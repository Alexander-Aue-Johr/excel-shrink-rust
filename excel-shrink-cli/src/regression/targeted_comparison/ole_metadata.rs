//! Observe OLE package identities and anchors without constructing a worksheet DOM.
//! Observations do not qualify an object or change capture coverage.
use crate::regression::{
    office_plan::{decode_attr, formula_worksheet_parts, resolve_part_target},
    streaming_xml::{self, Event},
    xml_views::{Attributes, Kind, Reader},
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::Path,
};
use zip::ZipArchive;

mod representations;

#[derive(Debug, Default, Serialize)]
pub(super) struct Catalogue {
    worksheet_part: String,
    namespaces: BTreeMap<String, BTreeSet<String>>,
    objects: Vec<Object>,
    relationships: BTreeMap<String, Relationship>,
    representations: Vec<representations::Representation>,
}

#[derive(Debug, Default, Serialize)]
struct Object {
    qualified_name: String,
    attributes: BTreeMap<String, String>,
    properties: Option<BTreeMap<String, String>>,
    anchor: BTreeMap<String, i64>,
    child_elements: BTreeSet<String>,
}

#[derive(Debug, Serialize)]
struct Relationship {
    kind: String,
    target: String,
    external: bool,
    bytes: Option<u64>,
    sha256: Option<String>,
}

fn attributes(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for attribute in Attributes::new(bytes) {
        let (key, value) = attribute?;
        let key = std::str::from_utf8(key)?.to_owned();
        ensure!(
            result.insert(key, decode_attr(value)?).is_none(),
            "Duplicate OLE metadata attribute"
        );
    }
    Ok(result)
}

/// Stream the worksheet body and retain only OLE records and their small anchor fields.
fn objects(input: impl Read) -> Result<(Vec<Object>, BTreeMap<String, BTreeSet<String>>)> {
    let mut result = Vec::<Object>::new();
    let mut namespaces = BTreeMap::<String, BTreeSet<String>>::new();
    let mut active = None;
    let mut marker = None;
    let mut coordinate: Option<(String, Vec<u8>)> = None;
    streaming_xml::scan(input, |event| {
        match event {
            Event::Tag(tag) => {
                if tag.kind != Kind::End
                    && (tag.depth == 0
                        || active.is_some()
                        || matches!(
                            tag.local(),
                            b"oleObjects"
                                | b"oleObject"
                                | b"AlternateContent"
                                | b"Choice"
                                | b"Fallback"
                        ))
                {
                    for attribute in Attributes::new(tag.attrs) {
                        let (key, value) = attribute?;
                        if key == b"xmlns" || key.starts_with(b"xmlns:") {
                            namespaces
                                .entry(std::str::from_utf8(key)?.to_owned())
                                .or_default()
                                .insert(decode_attr(value)?);
                        }
                    }
                }
                if tag.local() == b"oleObject" {
                    if tag.kind == Kind::End {
                        active = None;
                    } else {
                        ensure!(active.is_none(), "Nested OLE records");
                        result.push(Object {
                            qualified_name: std::str::from_utf8(tag.name)?.to_owned(),
                            attributes: attributes(tag.attrs)?,
                            ..Default::default()
                        });
                        active = (tag.kind == Kind::Start).then_some(result.len() - 1);
                    }
                    return Ok(());
                }
                let Some(index) = active else { return Ok(()) };
                let object = &mut result[index];
                if tag.kind != Kind::End {
                    object
                        .child_elements
                        .insert(std::str::from_utf8(tag.name)?.to_owned());
                }
                match tag.local() {
                    b"objectPr" if tag.kind != Kind::End => {
                        ensure!(object.properties.is_none(), "Multiple OLE property records");
                        object.properties = Some(attributes(tag.attrs)?);
                    }
                    b"from" | b"to" => {
                        marker = (tag.kind == Kind::Start)
                            .then(|| String::from_utf8_lossy(tag.local()).into_owned());
                    }
                    b"row" | b"col" | b"rowOff" | b"colOff" if marker.is_some() => {
                        if tag.kind == Kind::Start {
                            ensure!(coordinate.is_none(), "Nested OLE coordinates");
                            coordinate = Some((
                                format!(
                                    "{}.{}",
                                    marker.as_ref().unwrap(),
                                    std::str::from_utf8(tag.local())?
                                ),
                                Vec::new(),
                            ));
                        } else if tag.kind == Kind::End {
                            let (key, bytes) = coordinate
                                .take()
                                .context("OLE coordinate end without start")?;
                            let number = std::str::from_utf8(&bytes)?.trim().parse()?;
                            ensure!(
                                object.anchor.insert(key, number).is_none(),
                                "Duplicate OLE coordinate"
                            );
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(bytes) | Event::Cdata(bytes) => {
                if let Some((_, text)) = &mut coordinate {
                    ensure!(text.len() + bytes.len() <= 64, "Oversized OLE coordinate");
                    text.extend_from_slice(bytes);
                }
            }
        }
        Ok(())
    })?;
    ensure!(
        active.is_none() && coordinate.is_none(),
        "Incomplete OLE metadata"
    );
    Ok((result, namespaces))
}

fn read_relationships(
    archive: &mut ZipArchive<File>,
    part: &str,
) -> Result<BTreeMap<String, Relationship>> {
    let (directory, file) = part
        .rsplit_once('/')
        .context("Worksheet part has no directory")?;
    let name = format!("{directory}/_rels/{file}.rels");
    let mut entry = match archive.by_name(&name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(BTreeMap::new()),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        entry.size() <= 1024 * 1024,
        "Oversized worksheet relationship metadata"
    );
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    let mut reader = Reader::<64>::new(&bytes)?;
    let mut result = BTreeMap::new();
    while let Some(tag) = reader.next()? {
        if tag.local() != b"Relationship" || tag.kind == Kind::End {
            continue;
        }
        let id = decode_attr(tag.attr(b"Id")?.context("OLE relationship ID missing")?)?;
        let kind = decode_attr(
            tag.attr(b"Type")?
                .context("OLE relationship type missing")?,
        )?;
        let target = decode_attr(
            tag.attr(b"Target")?
                .context("OLE relationship target missing")?,
        )?;
        let mode = tag.attr(b"TargetMode")?;
        ensure!(
            mode.is_none_or(|value| value == b"Internal" || value == b"External"),
            "Unknown OLE relationship target mode"
        );
        let external = mode == Some(b"External");
        let target = if external {
            target
        } else {
            resolve_part_target(part, &target)?
        };
        ensure!(
            result
                .insert(
                    id,
                    Relationship {
                        kind,
                        target,
                        external,
                        bytes: None,
                        sha256: None
                    }
                )
                .is_none(),
            "Duplicate worksheet relationship ID"
        );
    }
    Ok(result)
}

/// Bind observations to existing local parts. External references are recorded, never fetched.
pub(super) fn inspect(path: &Path, sheet: &str) -> Result<Catalogue> {
    let mut archive = ZipArchive::new(File::open(path)?)?;
    let part = formula_worksheet_parts(&mut archive)?
        .into_iter()
        .find_map(|(name, part)| (name == sheet).then_some(part))
        .context("OLE metadata worksheet missing")?;
    let mut relationships = read_relationships(&mut archive, &part)?;
    if !relationships
        .values()
        .any(|relationship| relationship.kind.ends_with("/oleObject"))
    {
        return Ok(Catalogue {
            worksheet_part: part,
            ..Default::default()
        });
    }
    let (objects, namespaces) = objects(archive.by_name(&part)?)?;
    let representations = representations::inspect(&mut archive, &relationships)?;
    let used: BTreeSet<_> = objects
        .iter()
        .flat_map(|object| {
            object.attributes.get("r:id").into_iter().chain(
                object
                    .properties
                    .as_ref()
                    .and_then(|attrs| attrs.get("r:id")),
            )
        })
        .cloned()
        .collect();
    relationships.retain(|id, _| used.contains(id));
    ensure!(
        relationships.len() == used.len(),
        "OLE metadata references a missing relationship"
    );
    hash_local_parts(&mut archive, &mut relationships)?;
    Ok(Catalogue {
        worksheet_part: part,
        namespaces,
        objects,
        relationships,
        representations,
    })
}

fn hash_local_parts(
    archive: &mut ZipArchive<File>,
    relationships: &mut BTreeMap<String, Relationship>,
) -> Result<()> {
    for relationship in relationships
        .values_mut()
        .filter(|relationship| !relationship.external)
    {
        let mut entry = archive
            .by_name(&relationship.target)
            .context("OLE metadata part missing")?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 8192];
        loop {
            let count = entry.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        relationship.bytes = Some(entry.size());
        relationship.sha256 = Some(format!("{:x}", hash.finalize()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const OBJECTS: &str = r#"
        <oleObjects>
          <mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
            <mc:Choice Requires="x14">
              <oleObject shapeId="4099" progId="Document" r:id="object">
                <objectPr defaultSize="0" r:id="preview">
                  <anchor moveWithCells="1">
                    <from><xdr:col>0</xdr:col><xdr:row>0</xdr:row><xdr:colOff>0</xdr:colOff><xdr:rowOff>0</xdr:rowOff></from>
                    <to><xdr:col>7</xdr:col><xdr:row>50</xdr:row><xdr:colOff>295275</xdr:colOff><xdr:rowOff>57150</xdr:rowOff></to>
                  </anchor>
                </objectPr>
              </oleObject>
            </mc:Choice>
            <mc:Fallback><oleObject shapeId="4099" progId="Document" r:id="object"/></mc:Fallback>
          </mc:AlternateContent>
        </oleObjects>"#;

    fn workbook(path: &Path, metadata: &str, preview: bool, external: bool) -> Result<()> {
        let mut zip = zip::ZipWriter::new(File::create(path)?);
        let sheet = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"><sheetData><row r="1"><c r="A1"><v>7</v></c></row></sheetData>{metadata}</worksheet>"#
        );
        let mode = if external {
            " TargetMode=\"External\""
        } else {
            ""
        };
        let target = if external {
            "https://example.invalid/object"
        } else {
            "../embeddings/object.bin"
        };
        let preview_rel = if preview {
            r#"<Relationship Id="preview" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/preview.emf"/>"#
        } else {
            ""
        };
        let relationships = format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="object" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject" Target="{target}"{mode}/>{preview_rel}</Relationships>"#
        );
        for (name, bytes) in [
            (
                "xl/workbook.xml",
                r#"<workbook><sheets><sheet name="Data" r:id="sheet"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="sheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            ("xl/worksheets/sheet1.xml", &sheet),
            ("xl/worksheets/_rels/sheet1.xml.rels", &relationships),
            ("xl/embeddings/object.bin", "embedded payload"),
            ("xl/media/preview.emf", "preview bytes, metadata test only"),
        ] {
            zip.start_file(name, zip::write::FileOptions::default())?;
            zip.write_all(bytes.as_bytes())?;
        }
        zip.finish()?;
        Ok(())
    }

    #[test]
    fn observes_choice_fallback_anchor_and_part_hashes_without_qualifying() -> Result<()> {
        let root = std::env::temp_dir().join(format!("ole-observations-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root)?;
        let path = root.join("input.xlsx");
        workbook(&path, OBJECTS, true, false)?;
        let data = inspect(&path, "Data")?;
        assert_eq!(data.objects.len(), 2);
        assert_eq!(data.objects[0].anchor["to.row"], 50);
        assert_eq!(data.objects[0].anchor["to.colOff"], 295275);
        assert!(data.objects[1].properties.is_none());
        assert_eq!(data.relationships.len(), 2);
        assert_eq!(
            data.relationships["object"].target,
            "xl/embeddings/object.bin"
        );
        assert_eq!(
            data.relationships["object"].sha256.as_deref(),
            Some(format!("{:x}", Sha256::digest(b"embedded payload")).as_str())
        );
        assert!(data.namespaces["xmlns:r"]
            .contains("http://schemas.openxmlformats.org/officeDocument/2006/relationships"));
        assert!(serde_json::to_value(data)?.get("qualified").is_none());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn external_parts_are_recorded_without_access_and_missing_relations_fail() -> Result<()> {
        let root = std::env::temp_dir().join(format!("ole-external-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root)?;
        let path = root.join("input.xlsx");
        workbook(&path, OBJECTS, true, true)?;
        let data = inspect(&path, "Data")?;
        assert!(data.relationships["object"].external);
        assert!(data.relationships["object"].sha256.is_none());
        workbook(&path, OBJECTS, false, false)?;
        assert!(inspect(&path, "Data")
            .unwrap_err()
            .to_string()
            .contains("missing relationship"));
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn malformed_or_duplicate_coordinates_are_errors() {
        for input in [
            r#"<worksheet><oleObject><anchor><from><row>1</row><row>2</row></from></anchor></oleObject></worksheet>"#,
            r#"<worksheet><oleObject><anchor><from><row>unknown</row></from></anchor></oleObject></worksheet>"#,
        ] {
            assert!(objects(input.as_bytes()).is_err());
        }
    }
}
