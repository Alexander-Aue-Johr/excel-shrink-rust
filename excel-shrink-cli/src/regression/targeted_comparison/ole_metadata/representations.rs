//! Retain DrawingML/VML identities alongside worksheet OLE records.
//! These are observations, without a supported-frame or namespace-resolution claim.
use super::{attributes, hash_local_parts, read_relationships, Relationship};
use crate::regression::xml_views::{Kind, Reader};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::File, io::Read};
use zip::ZipArchive;

#[derive(Debug, Default, Serialize)]
pub(super) struct Representation {
    part: String,
    external: bool,
    sha256: Option<String>,
    namespace_declarations: BTreeMap<String, Vec<String>>,
    shapes: Vec<Shape>,
    relationships: BTreeMap<String, Relationship>,
}

#[derive(Debug, Serialize)]
struct Shape {
    qualified_name: String,
    attributes: BTreeMap<String, String>,
    compatibility_ids: Vec<String>,
    preview_relationship_ids: Vec<String>,
    legacy_anchors: Vec<String>,
}

fn read(part: String, bytes: &[u8]) -> Result<Representation> {
    let mut result = Representation {
        part,
        sha256: Some(format!("{:x}", Sha256::digest(bytes))),
        ..Default::default()
    };
    let mut reader = Reader::<128>::new(bytes)?;
    let mut active = None;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End {
            if active.is_some_and(|(_, depth)| tag.depth == depth) {
                active = None;
            }
            continue;
        }
        let attrs = attributes(tag.attrs)?;
        for (key, value) in &attrs {
            if key == "xmlns" || key.starts_with("xmlns:") {
                result
                    .namespace_declarations
                    .entry(key.clone())
                    .or_default()
                    .push(value.clone());
            }
        }
        if matches!(tag.name, b"xdr:cNvPr" | b"v:shape") {
            ensure!(active.is_none(), "Nested drawing identity records");
            result.shapes.push(Shape {
                qualified_name: std::str::from_utf8(tag.name)?.to_owned(),
                attributes: attrs,
                compatibility_ids: Vec::new(),
                preview_relationship_ids: Vec::new(),
                legacy_anchors: Vec::new(),
            });
            if tag.kind == Kind::Start {
                active = Some((result.shapes.len() - 1, tag.depth));
            }
            continue;
        }
        let Some((index, _)) = active else { continue };
        let shape = &mut result.shapes[index];
        match tag.name {
            b"a14:compatExt" => {
                shape.compatibility_ids.push(
                    attrs
                        .get("spid")
                        .context("Compatibility shape ID missing")?
                        .clone(),
                );
            }
            b"v:imagedata" => {
                shape.preview_relationship_ids.push(
                    attrs
                        .get("o:relid")
                        .context("VML image relationship missing")?
                        .clone(),
                );
            }
            b"x:Anchor" => {
                let anchor = reader.element(tag)?;
                shape
                    .legacy_anchors
                    .push(std::str::from_utf8(anchor.body)?.trim().to_owned());
            }
            _ => {}
        }
    }
    Ok(result)
}

pub(super) fn inspect(
    archive: &mut ZipArchive<File>,
    worksheet_relationships: &BTreeMap<String, Relationship>,
) -> Result<Vec<Representation>> {
    let mut result = Vec::new();
    for relationship in worksheet_relationships.values() {
        if !relationship.kind.ends_with("/drawing") && !relationship.kind.ends_with("/vmlDrawing") {
            continue;
        }
        // No network access is attempted for external representation targets.
        if relationship.external {
            result.push(Representation {
                part: relationship.target.clone(),
                external: true,
                ..Default::default()
            });
            continue;
        }
        let mut bytes = Vec::new();
        {
            let mut entry = archive.by_name(&relationship.target)?;
            ensure!(
                entry.size() <= 1024 * 1024,
                "Oversized OLE representation metadata"
            );
            entry.read_to_end(&mut bytes)?;
        }
        let mut representation = read(relationship.target.clone(), &bytes)?;
        let used: std::collections::BTreeSet<_> = representation
            .shapes
            .iter()
            .flat_map(|shape| &shape.preview_relationship_ids)
            .collect();
        let mut relationships = read_relationships(archive, &relationship.target)?;
        relationships.retain(|id, _| used.contains(id));
        ensure!(
            relationships.len() == used.len(),
            "VML preview relationship missing"
        );
        hash_local_parts(archive, &mut relationships)?;
        representation.relationships = relationships;
        result.push(representation);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture(path: &std::path::Path, include_preview_relationship: bool) -> Result<()> {
        let mut zip = zip::ZipWriter::new(File::create(path)?);
        let mut parts = vec![
            ("xl/drawings/drawing.vml", br#"<xml><v:shape id="_x0000_s2050"><v:imagedata o:relid="image"/></v:shape></xml>"#.as_slice()),
            ("xl/media/preview.emf", b"metadata fixture, not a native image"),
        ];
        if include_preview_relationship {
            parts.push(("xl/drawings/_rels/drawing.vml.rels", br#"<Relationships><Relationship Id="image" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/preview.emf"/></Relationships>"#));
        }
        for (name, bytes) in parts {
            zip.start_file(name, zip::write::FileOptions::default())?;
            zip.write_all(bytes)?;
        }
        zip.finish()?;
        Ok(())
    }

    #[test]
    fn binds_local_preview_hash_records_external_target_and_rejects_missing_link() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("ole-representations-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root)?;
        let path = root.join("input.zip");
        fixture(&path, true)?;
        let mut relationships = BTreeMap::new();
        for (id, target, external) in [
            ("local", "xl/drawings/drawing.vml", false),
            ("remote", "https://example.invalid/external.vml", true),
        ] {
            relationships.insert(id.to_owned(), Relationship {
                kind: "http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing".into(),
                target: target.into(), external, bytes: None, sha256: None,
            });
        }
        {
            let mut archive = ZipArchive::new(File::open(&path)?)?;
            let data = inspect(&mut archive, &relationships)?;
            assert_eq!(data.len(), 2);
            assert_eq!(
                data[0].relationships["image"].sha256.as_deref(),
                Some(
                    format!(
                        "{:x}",
                        Sha256::digest(b"metadata fixture, not a native image")
                    )
                    .as_str()
                )
            );
            assert!(data[1].external);
            assert!(data[1].sha256.is_none());
            assert!(data[1].relationships.is_empty());
        }
        fixture(&path, false)?;
        let mut archive = ZipArchive::new(File::open(&path)?)?;
        assert!(inspect(&mut archive, &relationships)
            .unwrap_err()
            .to_string()
            .contains("VML preview relationship missing"));
        drop(archive);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn records_drawing_identity_and_vml_preview_without_qualification() -> Result<()> {
        let drawing = read("drawing.xml".into(), br#"<xdr:wsDr><xdr:cNvPr id="2050" name="Object 2"><a14:compatExt spid="_x0000_s2050"/></xdr:cNvPr></xdr:wsDr>"#)?;
        assert_eq!(drawing.shapes[0].attributes["name"], "Object 2");
        assert_eq!(drawing.shapes[0].compatibility_ids, ["_x0000_s2050"]);
        let vml = read("drawing.vml".into(), br#"<xml xmlns:v="urn:schemas-microsoft-com:vml"><v:shape id="_x0000_s2050" style="margin-top:114.75pt"><v:imagedata o:relid="preview"/><x:ClientData><x:Anchor>1, 7, 7, 14, 2, 79, 14, 9</x:Anchor></x:ClientData></v:shape></xml>"#)?;
        assert_eq!(
            vml.shapes[0].attributes["id"],
            drawing.shapes[0].compatibility_ids[0]
        );
        assert_eq!(vml.shapes[0].preview_relationship_ids, ["preview"]);
        assert_eq!(vml.shapes[0].legacy_anchors, ["1, 7, 7, 14, 2, 79, 14, 9"]);
        assert!(serde_json::to_value(vml)?.get("qualified").is_none());
        Ok(())
    }

    #[test]
    fn rejects_missing_identity_fields_and_nested_shapes() {
        for bytes in [
            br#"<xml><v:shape><v:imagedata/></v:shape></xml>"#.as_slice(),
            br#"<xdr:wsDr><xdr:cNvPr><a14:compatExt/></xdr:cNvPr></xdr:wsDr>"#,
            br#"<xml><v:shape><v:shape/></v:shape></xml>"#,
        ] {
            assert!(read("test.xml".into(), bytes).is_err());
        }
    }
}
