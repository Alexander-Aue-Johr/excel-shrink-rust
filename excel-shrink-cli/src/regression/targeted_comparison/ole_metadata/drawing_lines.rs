//! Recognize effect-free horizontal line frames for independent native qualification.
use super::{attributes, namespaces::Namespaces};
use crate::regression::{
    targeted_comparison::capture_layout::Geometry,
    xml_views::{Kind, Reader},
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;

const DRAWING: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const MAIN: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

#[derive(Debug, Serialize)]
pub(in crate::regression::targeted_comparison) struct Frame {
    pub name: String,
    pub geometry: Geometry,
    pub stroke_points: f64,
}

struct Node {
    attrs: BTreeMap<String, String>,
    text: String,
}

fn exact(node: &Node, expected: &[(&str, &str)]) -> Result<()> {
    ensure!(
        node.attrs.len() == expected.len()
            && expected
                .iter()
                .all(|(key, value)| { node.attrs.get(*key).map(String::as_str) == Some(*value) }),
        "Unsupported line attributes"
    );
    Ok(())
}

fn number(node: &Node, key: &str) -> Result<u64> {
    let value = node
        .attrs
        .get(key)
        .context("Missing line coordinate")?
        .parse()?;
    ensure!(value <= (1_u64 << 52), "Unrepresentable line coordinate");
    Ok(value)
}

fn frame(nodes: BTreeMap<String, Node>) -> Result<Frame> {
    for (path, node) in &nodes {
        match path.as_str() {
            ""
            | "from"
            | "to"
            | "sp/nvSpPr"
            | "sp/nvSpPr/cNvSpPr"
            | "sp/spPr/xfrm"
            | "sp/spPr/prstGeom/avLst"
            | "sp/spPr/noFill"
            | "sp/spPr/ln/solidFill"
            | "sp/spPr/ln/round"
            | "clientData"
            | "sp/nvSpPr/cNvPr/extLst"
            | "sp/spPr/extLst"
            | "sp/spPr/extLst/ext/hiddenFill"
            | "sp/spPr/extLst/ext/hiddenFill/noFill" => exact(node, &[])?,
            "from/col" | "from/row" | "from/colOff" | "from/rowOff" | "to/col" | "to/row"
            | "to/colOff" | "to/rowOff" => {
                exact(node, &[])?;
                let _: u32 = node.text.trim().parse()?;
            }
            "sp" => {
                ensure!(
                    node.attrs.iter().all(|(key, value)| {
                        matches!(key.as_str(), "macro" | "textlink") && value.is_empty()
                    }),
                    "Active or unknown line shape property"
                );
            }
            "sp/nvSpPr/cNvPr" => {
                ensure!(
                    node.attrs.len() == 2
                        && node.attrs.contains_key("id")
                        && node.attrs.get("name").is_some_and(|name| !name.is_empty()),
                    "Unknown line identity property"
                );
                let _: u32 = node.attrs["id"].parse()?;
            }
            "sp/nvSpPr/cNvSpPr/spLocks" => exact(node, &[("noChangeShapeType", "1")])?,
            "sp/spPr" => {
                ensure!(
                    node.attrs.is_empty()
                        || node.attrs == BTreeMap::from([("bwMode".into(), "auto".into())]),
                    "Unknown line rendering property"
                );
            }
            "sp/spPr/xfrm/off" => {
                ensure!(node.attrs.len() == 2, "Unknown line offset property");
                number(node, "x")?;
                number(node, "y")?;
            }
            "sp/spPr/xfrm/ext" => {
                ensure!(
                    node.attrs.len() == 2 && number(node, "cx")? > 0 && number(node, "cy")? == 0,
                    "Expected a horizontal line"
                );
            }
            "sp/spPr/prstGeom" => exact(node, &[("prst", "line")])?,
            "sp/spPr/ln" => {
                ensure!(
                    node.attrs.len() == 1 && (1..=12700).contains(&number(node, "w")?),
                    "Unsupported line stroke width"
                );
            }
            "sp/spPr/ln/solidFill/srgbClr" => {
                let color = node.attrs.get("val").context("Missing line color")?;
                ensure!(
                    node.attrs.len() == 1
                        && color.len() == 6
                        && color.bytes().all(|byte| byte.is_ascii_hexdigit()),
                    "Unsupported line color"
                );
            }
            "sp/spPr/ln/headEnd" | "sp/spPr/ln/tailEnd" => {
                ensure!(
                    node.attrs.is_empty()
                        || node.attrs == BTreeMap::from([("type".into(), "none".into())]),
                    "Line arrows require a separate envelope"
                );
            }
            "sp/nvSpPr/cNvPr/extLst/ext" => {
                exact(node, &[("uri", "{FF2B5EF4-FFF2-40B4-BE49-F238E27FC236}")])?
            }
            "sp/nvSpPr/cNvPr/extLst/ext/creationId" => {
                ensure!(
                    node.attrs.len() == 1 && node.attrs.get("id").is_some_and(|id| id.len() <= 100),
                    "Unknown line creation metadata"
                );
            }
            "sp/spPr/extLst/ext" => {
                exact(node, &[("uri", "{909E8E84-426E-40DD-AFC4-6F175D3DCCD1}")])?
            }
            _ => anyhow::bail!("Unsupported line element {path}"),
        }
        ensure!(
            path.starts_with("from/") || path.starts_with("to/") || node.text.trim().is_empty(),
            "Unexpected line text"
        );
    }
    for path in [
        "from/col",
        "from/row",
        "from/colOff",
        "from/rowOff",
        "to/col",
        "to/row",
        "to/colOff",
        "to/rowOff",
        "sp/nvSpPr/cNvPr",
        "sp/nvSpPr/cNvSpPr",
        "sp/spPr/xfrm/off",
        "sp/spPr/xfrm/ext",
        "sp/spPr/prstGeom/avLst",
        "sp/spPr/noFill",
        "sp/spPr/ln/solidFill/srgbClr",
        "sp/spPr/ln/round",
        "sp/spPr/ln/headEnd",
        "sp/spPr/ln/tailEnd",
        "clientData",
    ] {
        ensure!(
            nodes.contains_key(path),
            "Missing line template element {path}"
        );
    }
    let offset = &nodes["sp/spPr/xfrm/off"];
    let extent = &nodes["sp/spPr/xfrm/ext"];
    Ok(Frame {
        name: nodes["sp/nvSpPr/cNvPr"].attrs["name"].clone(),
        geometry: Geometry {
            left: number(offset, "x")? as f64 / 12700.0,
            top: number(offset, "y")? as f64 / 12700.0,
            width: number(extent, "cx")? as f64 / 12700.0,
            height: 0.0,
        },
        stroke_points: number(&nodes["sp/spPr/ln"], "w")? as f64 / 12700.0,
    })
}

pub(super) fn read(bytes: &[u8]) -> Result<Vec<Frame>> {
    let mut reader = Reader::<128>::new(bytes)?;
    let mut namespaces = Namespaces::default();
    let mut path = Vec::new();
    let mut nodes = BTreeMap::new();
    let mut frames = Vec::new();
    let mut root_seen = false;
    while let Some(tag) = reader.next()? {
        namespaces.observe(tag)?;
        if tag.kind == Kind::End {
            if tag.depth == 1 {
                frames.push(frame(std::mem::take(&mut nodes))?);
            }
            path.pop();
            continue;
        }
        let local = std::str::from_utf8(tag.local())?;
        let expected_namespace = match local {
            "wsDr" | "twoCellAnchor" | "from" | "to" | "col" | "row" | "colOff" | "rowOff"
            | "sp" | "nvSpPr" | "cNvPr" | "cNvSpPr" | "spPr" | "clientData" => DRAWING,
            "creationId" => "http://schemas.microsoft.com/office/drawing/2014/main",
            "hiddenFill" => "http://schemas.microsoft.com/office/drawing/2010/main",
            _ => MAIN,
        };
        ensure!(
            namespaces.element(tag.name)?.as_deref() == Some(expected_namespace),
            "Unsupported line namespace"
        );
        path.push(local.to_owned());
        let mut attrs = attributes(tag.attrs)?;
        attrs.retain(|name, _| name != "xmlns" && !name.starts_with("xmlns:"));
        if local == "srgbClr" && attrs.contains_key("a14:legacySpreadsheetColorIndex") {
            // Excel's legacy palette hint changes no line geometry. Keep the
            // explicit RGB value and require the exact ignorable extension scope.
            ensure!(
                namespaces
                    .element(b"a14:legacySpreadsheetColorIndex")?
                    .as_deref()
                    == Some("http://schemas.microsoft.com/office/drawing/2010/main")
                    && namespaces.element(b"mc:Ignorable")?.as_deref()
                        == Some("http://schemas.openxmlformats.org/markup-compatibility/2006")
                    && attrs.get("mc:Ignorable").map(String::as_str) == Some("a14")
                    && attrs["a14:legacySpreadsheetColorIndex"]
                        .parse::<u8>()
                        .is_ok_and(|index| index <= 65),
                "Unsupported legacy line color metadata"
            );
            attrs.remove("a14:legacySpreadsheetColorIndex");
            attrs.remove("mc:Ignorable");
        }
        if tag.depth == 0 {
            ensure!(
                !root_seen && local == "wsDr" && attrs.is_empty(),
                "Expected one drawing root"
            );
            root_seen = true;
        } else {
            ensure!(
                tag.depth != 1 || (local == "twoCellAnchor" && tag.kind == Kind::Start),
                "Expected two-cell line anchor"
            );
            let key = path.iter().skip(2).cloned().collect::<Vec<_>>().join("/");
            let text = if tag.kind == Kind::Start {
                let rest = &bytes[tag.end..];
                let end = rest
                    .iter()
                    .position(|byte| *byte == b'<')
                    .context("Unterminated line element")?;
                std::str::from_utf8(&rest[..end])?.to_owned()
            } else {
                String::new()
            };
            ensure!(
                nodes.insert(key, Node { attrs, text }).is_none(),
                "Duplicate line template element"
            );
        }
        if tag.kind == Kind::Empty {
            path.pop();
        }
    }
    ensure!(
        !frames.is_empty() && path.is_empty(),
        "Missing complete line anchors"
    );
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
      <xdr:twoCellAnchor>
        <xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>95250</xdr:rowOff></xdr:from>
        <xdr:to><xdr:col>3</xdr:col><xdr:colOff>800100</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>95250</xdr:rowOff></xdr:to>
        <xdr:sp><xdr:nvSpPr><xdr:cNvPr id="2" name="Line 1"/><xdr:cNvSpPr/></xdr:nvSpPr>
          <xdr:spPr><a:xfrm><a:off x="0" y="323850"/><a:ext cx="9925050" cy="0"/></a:xfrm>
            <a:prstGeom prst="line"><a:avLst/></a:prstGeom><a:noFill/>
            <a:ln w="10795"><a:solidFill><a:srgbClr val="000000"/></a:solidFill><a:round/><a:headEnd/><a:tailEnd/></a:ln>
          </xdr:spPr>
        </xdr:sp><xdr:clientData/>
      </xdr:twoCellAnchor>
    </xdr:wsDr>"#;

    #[test]
    fn recognizes_the_thin_horizontal_line_frame() -> Result<()> {
        let frames = read(LINE.as_bytes())?;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].name, "Line 1");
        assert_eq!(
            frames[0].geometry,
            Geometry {
                left: 0.0,
                top: 25.5,
                width: 781.5,
                height: 0.0,
            }
        );
        assert_eq!(frames[0].stroke_points, 0.85);
        Ok(())
    }

    #[test]
    fn legacy_palette_hint_keeps_geometry_and_rejects_unknown_metadata() -> Result<()> {
        let color = r#"<a:srgbClr val="000000" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="a14" xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main" a14:legacySpreadsheetColorIndex="64"/>"#;
        let xml = LINE.replace(r#"<a:srgbClr val="000000"/>"#, color);
        assert_eq!(
            read(xml.as_bytes())?[0].geometry,
            read(LINE.as_bytes())?[0].geometry
        );
        for changed in [
            xml.replace(
                "legacySpreadsheetColorIndex=\"64\"",
                "legacySpreadsheetColorIndex=\"66\"",
            ),
            xml.replace("drawing/2010/main", "drawing/2010/unknown"),
            xml.replace("mc:Ignorable=\"a14\"", "mc:Ignorable=\"other\""),
        ] {
            assert!(read(changed.as_bytes()).is_err());
        }
        Ok(())
    }

    /// bevoelkerungsbewegung-2010110157005.xlsx, sheet 2.11, Line 1.
    /// The unsupported palette hint formerly forced A1:XFD1048576 coverage.
    #[test]
    #[ignore = "requires LEGACY_LINE_FIXTURE pointing to the retained XLSX"]
    fn retained_bevoelkerungsbewegung_palette_line_is_bounded() -> Result<()> {
        let path = std::env::var("LEGACY_LINE_FIXTURE")?;
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
        let mut xml = Vec::new();
        std::io::Read::read_to_end(&mut archive.by_name("xl/drawings/drawing7.xml")?, &mut xml)?;
        let frames = read(&xml)?;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].name, "Line 1");
        assert_eq!(frames[0].geometry.height, 0.0);
        assert_eq!(frames[0].stroke_points, 0.75);
        Ok(())
    }

    #[test]
    fn unsupported_effects_geometry_namespaces_and_arrows_remain_unqualified() {
        for (before, after) in [
            ("<a:xfrm>", "<a:xfrm rot=\"60000\">"),
            ("<a:xfrm>", "<a:xfrm flipH=\"1\">"),
            ("<a:headEnd/>", "<a:headEnd type=\"triangle\"/>"),
            ("<a:tailEnd/>", "<a:tailEnd type=\"arrow\"/>"),
            ("w=\"10795\"", "w=\"12701\""),
            ("cy=\"0\"", "cy=\"12700\""),
            ("prst=\"line\"", "prst=\"rect\""),
            ("<a:noFill/>", "<a:noFill/><a:effectLst/>"),
            ("<a:round/>", "<a:round/><a:unknown/>"),
            ("drawingml/2006/main", "drawingml/2006/foreign"),
        ] {
            assert!(
                read(LINE.replace(before, after).as_bytes()).is_err(),
                "{after}"
            );
        }
    }
}
