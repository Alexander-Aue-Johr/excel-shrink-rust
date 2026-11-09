//! Correlate observed identities, cached previews and anchors without qualifying rendering.
use super::{
    namespaces,
    representations::{Representation, Shape},
    vml_geometry, Object, Relationship,
};
use crate::regression::targeted_comparison::capture_layout::Geometry;
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const DRAWING: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const RELATIONSHIP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const EMU_PER_PIXEL: i64 = 9525;

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum Observation {
    Matched {
        shape_id: String,
        drawing_name: String,
        vml_part: String,
        preview_sha256: String,
        style_geometry: Geometry,
    },
    Unresolved {
        shape_id: Option<String>,
        reason: String,
    },
}

fn only<T>(mut items: Vec<T>, description: &str) -> Result<T> {
    ensure!(
        items.len() == 1,
        "Expected one {description}, observed {}",
        items.len()
    );
    Ok(items.remove(0))
}

fn namespace(observation: &namespaces::Observation, name: &str, expected: &str) -> bool {
    observation.element_namespaces.get(name) == Some(&BTreeSet::from([Some(expected.into())]))
}

fn local_part<'a>(
    relationships: &'a BTreeMap<String, Relationship>,
    id: &str,
    kind: &str,
) -> Result<&'a Relationship> {
    let part = relationships
        .get(id)
        .context("Object relationship missing")?;
    ensure!(
        part.element_namespace.as_deref()
            == Some("http://schemas.openxmlformats.org/package/2006/relationships")
            && !part.external
            && part.kind == format!("{RELATIONSHIP}/{kind}")
            && part.sha256.is_some()
            && part.bytes.is_some_and(|bytes| bytes > 0),
        "Object part is external, unsupported or unhashed"
    );
    Ok(part)
}

fn worksheet_record<'a>(records: &[&'a Object]) -> Result<&'a Object> {
    ensure!(records.len() <= 2, "Ambiguous OLE records for shape ID");
    let object = only(
        records
            .iter()
            .copied()
            .filter(|object| object.properties.is_some())
            .collect(),
        "anchored OLE record",
    )?;
    for record in records {
        ensure!(
            record.attributes == object.attributes
                && namespace(&record.resolved_namespaces, "oleObject", MAIN),
            "OLE branch identity or namespace differs"
        );
        ensure!(
            record.resolved_namespaces.relationship_namespaces
                == BTreeSet::from([Some(RELATIONSHIP.into())]),
            "OLE relationship namespace differs"
        );
    }
    for (name, bindings) in &object.resolved_namespaces.element_namespaces {
        let expected = match name.as_str() {
            "oleObject" | "objectPr" | "anchor" | "from" | "to" => MAIN,
            "xdr:row" | "xdr:col" | "xdr:rowOff" | "xdr:colOff" => DRAWING,
            _ => anyhow::bail!("Unknown OLE anchor element"),
        };
        ensure!(
            *bindings == BTreeSet::from([Some(expected.into())]),
            "OLE anchor namespace differs"
        );
    }
    Ok(object)
}

fn drawing_name<'a>(
    representations: &'a [Representation],
    id: &str,
    legacy_id: &str,
) -> Result<&'a str> {
    let drawing = only(
        representations
            .iter()
            .flat_map(|part| part.shapes.iter().map(move |shape| (part, shape)))
            .filter(|(_, shape)| {
                shape.qualified_name == "xdr:cNvPr"
                    && shape.attributes.get("id").is_some_and(|value| value == id)
            })
            .collect(),
        "DrawingML identity",
    )?;
    ensure!(
        !drawing.0.external
            && namespace(&drawing.1.resolved_namespaces, "xdr:cNvPr", DRAWING)
            && namespace(
                &drawing.1.resolved_namespaces,
                "a14:compatExt",
                "http://schemas.microsoft.com/office/drawing/2010/main"
            )
            && drawing.1.compatibility_ids == [legacy_id],
        "DrawingML identity namespace or compatibility ID differs"
    );
    drawing
        .1
        .attributes
        .get("name")
        .filter(|name| !name.is_empty())
        .map(String::as_str)
        .context("DrawingML object name absent")
}

fn vml_identity<'a>(
    representations: &'a [Representation],
    legacy_id: &str,
) -> Result<(&'a Representation, &'a Shape)> {
    let (vml_part, vml) = only(
        representations
            .iter()
            .flat_map(|part| part.shapes.iter().map(move |shape| (part, shape)))
            .filter(|(_, shape)| {
                shape.qualified_name == "v:shape"
                    && shape.attributes.get("id").map(String::as_str) == Some(legacy_id)
            })
            .collect(),
        "VML identity",
    )?;
    ensure!(
        !vml_part.external
            && namespace(
                &vml.resolved_namespaces,
                "v:shape",
                "urn:schemas-microsoft-com:vml"
            )
            && namespace(
                &vml.resolved_namespaces,
                "v:imagedata",
                "urn:schemas-microsoft-com:vml"
            )
            && namespace(
                &vml.resolved_namespaces,
                "x:Anchor",
                "urn:schemas-microsoft-com:office:excel"
            )
            && vml.resolved_namespaces.relationship_namespaces
                == BTreeSet::from([Some("urn:schemas-microsoft-com:office:office".into())]),
        "VML identity or relationship namespace differs"
    );
    Ok((vml_part, vml))
}

fn verify_anchor(object: &Object, vml: &Shape) -> Result<()> {
    let anchor = only(vml.legacy_anchors.iter().collect(), "VML anchor")?;
    let numbers: Vec<i64> = anchor
        .split(',')
        .map(|number| number.trim().parse())
        .collect::<std::result::Result<_, _>>()?;
    let mut expected = Vec::new();
    for key in [
        "from.col",
        "from.colOff",
        "from.row",
        "from.rowOff",
        "to.col",
        "to.colOff",
        "to.row",
        "to.rowOff",
    ] {
        let value = *object
            .anchor
            .get(key)
            .context("OLE anchor coordinate missing")?;
        ensure!(value >= 0, "Negative OLE anchor coordinate");
        if key.ends_with("Off") {
            ensure!(
                value % EMU_PER_PIXEL == 0,
                "OLE anchor offset has no exact legacy pixel representation"
            );
            expected.push(value / EMU_PER_PIXEL);
        } else {
            ensure!(
                value
                    < if key.ends_with("row") {
                        1_048_576
                    } else {
                        16_384
                    },
                "OLE anchor is outside the worksheet grid"
            );
            expected.push(value);
        }
    }
    ensure!(
        object.anchor.len() == 8 && numbers == expected,
        "Worksheet and VML anchors differ"
    );
    ensure!(
        (expected[4], expected[5]) >= (expected[0], expected[1])
            && (expected[6], expected[7]) >= (expected[2], expected[3]),
        "Inverted OLE anchor"
    );
    Ok(())
}

fn verify(
    id: &str,
    records: &[&Object],
    relationships: &BTreeMap<String, Relationship>,
    representations: &[Representation],
) -> Result<Observation> {
    id.parse::<u32>().context("Invalid OLE shape ID")?;
    let object = worksheet_record(records)?;
    let payload_id = object
        .attributes
        .get("r:id")
        .context("OLE payload relationship absent")?;
    local_part(relationships, payload_id, "oleObject")?;
    let properties = object.properties.as_ref().unwrap();
    let preview_id = properties
        .get("r:id")
        .context("OLE preview relationship absent")?;
    let preview = local_part(relationships, preview_id, "image")?;
    let legacy_id = format!("_x0000_s{id}");
    let drawing_name = drawing_name(representations, id, &legacy_id)?;
    let (vml_part, vml) = vml_identity(representations, &legacy_id)?;
    let vml_preview_id = only(
        vml.preview_relationship_ids.iter().collect(),
        "VML preview relationship",
    )?;
    let vml_preview = local_part(&vml_part.relationships, vml_preview_id, "image")?;
    ensure!(
        vml_preview.target == preview.target && vml_preview.sha256 == preview.sha256,
        "Worksheet and VML preview identities differ"
    );
    verify_anchor(object, vml)?;
    let geometry = match vml.style_frame {
        vml_geometry::Observation::ParsedPoints { geometry } => geometry,
        _ => anyhow::bail!("VML point frame unresolved"),
    };
    Ok(Observation::Matched {
        shape_id: id.into(),
        drawing_name: drawing_name.into(),
        vml_part: vml_part.part.clone(),
        preview_sha256: preview.sha256.clone().unwrap(),
        style_geometry: geometry,
    })
}

pub(super) fn inspect(
    objects: &[Object],
    relationships: &BTreeMap<String, Relationship>,
    representations: &[Representation],
) -> Vec<Observation> {
    let mut groups = BTreeMap::<String, Vec<&Object>>::new();
    let mut observations = Vec::new();
    for object in objects {
        match object.attributes.get("shapeId") {
            Some(id) => groups.entry(id.clone()).or_default().push(object),
            None => observations.push(Observation::Unresolved {
                shape_id: None,
                reason: "OLE shape ID absent".into(),
            }),
        }
    }
    for (id, records) in groups {
        observations.push(
            verify(&id, &records, relationships, representations).unwrap_or_else(|error| {
                Observation::Unresolved {
                    shape_id: Some(id),
                    reason: error.to_string(),
                }
            }),
        );
    }
    observations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relationship(kind: &str, target: &str) -> Relationship {
        Relationship {
            element_namespace: Some(
                "http://schemas.openxmlformats.org/package/2006/relationships".into(),
            ),
            kind: format!("{RELATIONSHIP}/{kind}"),
            target: target.into(),
            external: false,
            bytes: Some(1),
            sha256: Some("a".repeat(64)),
        }
    }

    fn fixture() -> Result<(
        Vec<Object>,
        BTreeMap<String, Relationship>,
        Vec<Representation>,
    )> {
        let xml = format!(
            r#"<worksheet xmlns="{MAIN}" xmlns:r="{RELATIONSHIP}" xmlns:xdr="{DRAWING}"><oleObject shapeId="2050" r:id="payload"><objectPr r:id="preview"><anchor><from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>2</xdr:row><xdr:rowOff>0</xdr:rowOff></from><to><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>4</xdr:row><xdr:rowOff>0</xdr:rowOff></to></anchor></objectPr></oleObject><oleObject shapeId="2050" r:id="payload"/></worksheet>"#
        );
        let (objects, _) = super::super::objects(xml.as_bytes())?;
        let relationships = BTreeMap::from([
            (
                "payload".into(),
                relationship("oleObject", "xl/embeddings/object.bin"),
            ),
            (
                "preview".into(),
                relationship("image", "xl/media/preview.emf"),
            ),
        ]);
        let drawing_xml = format!(
            r#"<xdr:wsDr xmlns:xdr="{DRAWING}" xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main"><xdr:cNvPr id="2050" name="Object 2"><a14:compatExt spid="_x0000_s2050"/></xdr:cNvPr></xdr:wsDr>"#
        );
        let drawing =
            super::super::representations::read("drawing.xml".into(), drawing_xml.as_bytes())?;
        let mut vml = super::super::representations::read("drawing.vml".into(), br#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape id="_x0000_s2050" style="position:absolute;margin-left:0;margin-top:0;width:10pt;height:20pt"><v:imagedata o:relid="image"/><x:ClientData><x:Anchor>1,0,2,0,3,0,4,0</x:Anchor></x:ClientData></v:shape></xml>"#)?;
        vml.relationships.insert(
            "image".into(),
            relationship("image", "xl/media/preview.emf"),
        );
        Ok((objects, relationships, vec![drawing, vml]))
    }

    #[test]
    fn correlates_unique_ids_namespaces_previews_and_exact_anchors() -> Result<()> {
        let (objects, relationships, representations) = fixture()?;
        let result = inspect(&objects, &relationships, &representations);
        assert_eq!(result.len(), 1);
        match &result[0] {
            Observation::Matched {
                shape_id,
                drawing_name,
                style_geometry,
                ..
            } => {
                assert_eq!(shape_id, "2050");
                assert_eq!(drawing_name, "Object 2");
                assert_eq!(style_geometry.width, 10.0);
            }
            _ => anyhow::bail!("Expected matched correspondence"),
        }
        assert!(serde_json::to_value(&result[0])?.get("qualified").is_none());
        Ok(())
    }

    #[test]
    fn conflicts_ambiguity_and_unknown_namespaces_remain_unresolved() -> Result<()> {
        for case in [
            "preview",
            "anchor",
            "external",
            "relationship-namespace",
            "compatibility-namespace",
            "branch",
            "duplicate",
            "fractional-offset",
            "outside-grid",
            "inverted",
        ] {
            let (mut objects, mut relationships, mut representations) = fixture()?;
            match case {
                "preview" => {
                    representations[1]
                        .relationships
                        .get_mut("image")
                        .unwrap()
                        .sha256 = Some("b".repeat(64))
                }
                "anchor" => {
                    representations[1].shapes[0].legacy_anchors[0] = "1,0,2,0,3,0,5,0".into()
                }
                "external" => relationships.get_mut("preview").unwrap().external = true,
                "relationship-namespace" => {
                    relationships.get_mut("preview").unwrap().element_namespace =
                        Some("foreign".into())
                }
                "compatibility-namespace" => {
                    representations[0].shapes[0]
                        .resolved_namespaces
                        .element_namespaces
                        .insert(
                            "a14:compatExt".into(),
                            BTreeSet::from([Some("foreign".into())]),
                        );
                }
                "branch" => {
                    objects[1].attributes.insert("r:id".into(), "other".into());
                }
                "duplicate" => representations.push(fixture()?.2.remove(1)),
                "fractional-offset" => {
                    objects[0].anchor.insert("from.colOff".into(), 1);
                }
                "outside-grid" => {
                    objects[0].anchor.insert("to.row".into(), 1_048_576);
                }
                "inverted" => {
                    objects[0].anchor.insert("from.row".into(), 5);
                    representations[1].shapes[0].legacy_anchors[0] = "1,0,5,0,3,0,4,0".into();
                }
                _ => unreachable!(),
            }
            assert!(
                matches!(
                    inspect(&objects, &relationships, &representations)[0],
                    Observation::Unresolved { .. }
                ),
                "{case}"
            );
        }
        Ok(())
    }
}
