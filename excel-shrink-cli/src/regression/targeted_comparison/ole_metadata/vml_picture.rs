//! Recognize the closed, effect-free Office picture template before using its frame.
use super::{attributes, namespaces::Namespaces, vml_geometry};
use crate::regression::xml_views::{Kind, Reader};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;

const FORMULAS: [&str; 12] = [
    "if lineDrawn pixelLineWidth 0",
    "sum @0 1 0",
    "sum 0 0 @1",
    "prod @2 1 2",
    "prod @3 21600 pixelWidth",
    "prod @3 21600 pixelHeight",
    "sum @0 0 1",
    "prod @6 1 2",
    "prod @7 21600 pixelWidth",
    "sum @8 21600 0",
    "prod @7 21600 pixelHeight",
    "sum @10 21600 0",
];

#[derive(Debug, Default, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum Observation {
    #[default]
    NotApplicable,
    RecognizedTemplate,
    Unsupported {
        reason: String,
    },
}

fn expected_namespace(name: &str) -> Result<&'static str> {
    Ok(match name.split_once(':').map(|(prefix, _)| prefix) {
        Some("v") => "urn:schemas-microsoft-com:vml",
        Some("o") => "urn:schemas-microsoft-com:office:office",
        Some("x") => "urn:schemas-microsoft-com:office:excel",
        None => "",
        _ => anyhow::bail!("Unknown VML namespace prefix"),
    })
}

fn exact(attrs: &BTreeMap<String, String>, expected: &[(&str, &str)]) -> Result<()> {
    ensure!(
        attrs.len() == expected.len()
            && expected
                .iter()
                .all(|(key, value)| attrs.get(*key).map(String::as_str) == Some(*value)),
        "Unsupported VML template attributes"
    );
    Ok(())
}

fn shape(attrs: &BTreeMap<String, String>) -> Result<()> {
    ensure!(
        attrs.keys().all(|key| matches!(
            key.as_str(),
            "id" | "type"
                | "style"
                | "filled"
                | "fillcolor"
                | "stroked"
                | "strokecolor"
                | "o:insetmode"
        )),
        "Unknown VML picture attribute"
    );
    attrs
        .get("id")
        .and_then(|id| id.strip_prefix("_x0000_s"))
        .context("Unsupported VML picture ID")?
        .parse::<u32>()?;
    ensure!(
        attrs.get("type").map(String::as_str) == Some("#_x0000_t75"),
        "Unsupported VML picture type"
    );
    ensure!(
        matches!(
            vml_geometry::observe(attrs.get("style").map(String::as_str)),
        vml_geometry::Observation::ParsedPoints { geometry } if !geometry.empty()
        ),
        "Unsupported VML picture style"
    );
    for (key, expected) in [
        ("filled", "t"),
        ("fillcolor", "window [65]"),
        ("strokecolor", "windowText [64]"),
        ("o:insetmode", "auto"),
    ] {
        ensure!(
            attrs.get(key).map(String::as_str) == Some(expected),
            "Unsupported VML picture appearance"
        );
    }
    ensure!(
        attrs
            .get("stroked")
            .is_none_or(|value| value == "t" || value == "f"),
        "Unsupported VML picture stroke"
    );
    Ok(())
}

fn validate_element(
    parent: &str,
    name: &str,
    attrs: &BTreeMap<String, String>,
    formulas: &mut Vec<String>,
) -> Result<()> {
    match (parent, name) {
        ("", "xml") => exact(attrs, &[]),
        ("xml", "o:shapelayout") => exact(attrs, &[("v:ext", "edit")]),
        ("o:shapelayout", "o:idmap") => {
            ensure!(
                attrs.len() == 2 && attrs.get("v:ext").map(String::as_str) == Some("edit"),
                "Unsupported VML ID map"
            );
            attrs
                .get("data")
                .context("VML ID map data missing")?
                .parse::<u32>()?;
            Ok(())
        }
        ("xml", "v:shapetype") => exact(
            attrs,
            &[
                ("id", "_x0000_t75"),
                ("coordsize", "21600,21600"),
                ("o:spt", "75"),
                ("o:preferrelative", "t"),
                ("path", "m@4@5l@4@11@9@11@9@5xe"),
                ("filled", "f"),
                ("stroked", "f"),
            ],
        ),
        ("v:shapetype", "v:stroke") => exact(attrs, &[("joinstyle", "miter")]),
        ("v:shapetype", "v:formulas") => exact(attrs, &[]),
        ("v:formulas", "v:f") => {
            ensure!(
                attrs.len() == 1,
                "Unsupported VML picture formula attributes"
            );
            formulas.push(
                attrs
                    .get("eqn")
                    .context("VML picture formula missing")?
                    .clone(),
            );
            Ok(())
        }
        ("v:shapetype", "v:path") => exact(
            attrs,
            &[
                ("o:extrusionok", "f"),
                ("gradientshapeok", "t"),
                ("o:connecttype", "rect"),
            ],
        ),
        ("v:shapetype", "o:lock") => exact(attrs, &[("v:ext", "edit"), ("aspectratio", "t")]),
        ("xml", "v:shape") => shape(attrs),
        ("v:shape", "v:fill") => exact(attrs, &[("color2", "window [65]")]),
        ("v:shape", "v:imagedata") => {
            ensure!(
                attrs.len() == 2
                    && attrs.get("o:title").is_some_and(String::is_empty)
                    && attrs.get("o:relid").is_some_and(|id| !id.is_empty()),
                "Unsupported VML preview properties"
            );
            Ok(())
        }
        ("v:shape", "x:ClientData") => exact(attrs, &[("ObjectType", "Pict")]),
        ("x:ClientData", "x:SizeWithCells" | "x:Anchor" | "x:CF" | "x:AutoPict") => {
            exact(attrs, &[])
        }
        _ => anyhow::bail!("Unsupported VML picture element {name} inside {parent}"),
    }
}

fn validate(bytes: &[u8]) -> Result<()> {
    let mut reader = Reader::<128>::new(bytes)?;
    let mut scope = Namespaces::default();
    let mut path = Vec::<String>::new();
    let mut formulas = Vec::new();
    let mut shape_types = 0;
    let mut type_elements = BTreeMap::<String, usize>::new();
    let mut shape_elements = Vec::<BTreeMap<String, usize>>::new();
    while let Some(tag) = reader.next()? {
        scope.observe(tag)?;
        if tag.kind == Kind::End {
            path.truncate(tag.depth);
            continue;
        }
        let name = std::str::from_utf8(tag.name)?;
        ensure!(
            scope.element(tag.name)?.as_deref() == Some(expected_namespace(name)?),
            "VML picture element namespace differs"
        );
        let mut attrs = attributes(tag.attrs)?;
        attrs.retain(|key, _| key != "xmlns" && !key.starts_with("xmlns:"));
        for key in attrs.keys().filter(|key| key.contains(':')) {
            ensure!(
                scope.element(key.as_bytes())?.as_deref() == Some(expected_namespace(key)?),
                "VML picture attribute namespace differs"
            );
        }
        ensure!(path.len() == tag.depth, "Unsupported VML picture nesting");
        validate_element(
            path.last().map(String::as_str).unwrap_or(""),
            name,
            &attrs,
            &mut formulas,
        )?;
        if path.last().is_some_and(|parent| parent == "v:shapetype") {
            *type_elements.entry(name.into()).or_default() += 1;
        }
        if name == "v:shape" {
            shape_elements.push(BTreeMap::new());
        } else if path.get(1).is_some_and(|parent| parent == "v:shape") {
            *shape_elements
                .last_mut()
                .unwrap()
                .entry(name.into())
                .or_default() += 1;
        }
        shape_types += usize::from(name == "v:shapetype");
        if name == "x:Anchor" || name == "x:CF" {
            ensure!(tag.kind == Kind::Start, "VML picture text missing");
            let body = reader.element(tag)?;
            let text = std::str::from_utf8(body.body)?.trim();
            if name == "x:CF" {
                ensure!(text == "Pict", "Unsupported VML picture clipboard format");
            } else {
                let coordinates: Vec<i64> = text
                    .split(',')
                    .map(|value| value.trim().parse())
                    .collect::<std::result::Result<_, _>>()?;
                ensure!(coordinates.len() == 8, "VML picture anchor is incomplete");
            }
        } else if tag.kind == Kind::Start {
            path.push(name.into());
        }
    }
    ensure!(
        path.is_empty()
            && shape_types == 1
            && !shape_elements.is_empty()
            && formulas.iter().map(String::as_str).eq(FORMULAS),
        "VML picture template is incomplete or ambiguous"
    );
    ensure!(
        ["v:stroke", "v:formulas", "v:path", "o:lock"]
            .iter()
            .all(|name| type_elements.get(*name) == Some(&1)),
        "VML picture type has missing or repeated definitions"
    );
    for elements in shape_elements {
        ensure!(
            [
                "v:fill",
                "v:imagedata",
                "x:ClientData",
                "x:SizeWithCells",
                "x:Anchor",
                "x:CF"
            ]
            .iter()
            .all(|name| elements.get(*name) == Some(&1))
                && elements.get("x:AutoPict").is_none_or(|count| *count == 1),
            "VML picture has missing or repeated properties"
        );
    }
    Ok(())
}

pub(super) fn observe(bytes: &[u8]) -> Observation {
    match validate(bytes) {
        Ok(()) => Observation::RecognizedTemplate,
        Err(error) => Observation::Unsupported {
            reason: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture() -> String {
        let formulas = FORMULAS
            .iter()
            .map(|formula| format!(r#"<v:f eqn="{formula}"/>"#))
            .collect::<String>();
        format!(
            r##"
<xml xmlns:v="urn:schemas-microsoft-com:vml"
     xmlns:o="urn:schemas-microsoft-com:office:office"
     xmlns:x="urn:schemas-microsoft-com:office:excel">
  <o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="2"/></o:shapelayout>
  <v:shapetype id="_x0000_t75" coordsize="21600,21600" o:spt="75"
      o:preferrelative="t" path="m@4@5l@4@11@9@11@9@5xe" filled="f" stroked="f">
    <v:stroke joinstyle="miter"/>
    <v:formulas>{formulas}</v:formulas>
    <v:path o:extrusionok="f" gradientshapeok="t" o:connecttype="rect"/>
    <o:lock v:ext="edit" aspectratio="t"/>
  </v:shapetype>
  <v:shape id="_x0000_s2050" type="#_x0000_t75"
      style="position:absolute;margin-left:0;margin-top:0;width:10pt;height:20pt"
      filled="t" fillcolor="window [65]" strokecolor="windowText [64]" o:insetmode="auto">
    <v:fill color2="window [65]"/>
    <v:imagedata o:relid="image" o:title=""/>
    <x:ClientData ObjectType="Pict">
      <x:SizeWithCells/><x:Anchor>1,0,2,0,3,0,4,0</x:Anchor><x:CF>Pict</x:CF>
    </x:ClientData>
  </v:shape>
</xml>"##
        )
    }

    #[test]
    fn recognizes_only_the_complete_static_picture_template() {
        assert!(matches!(
            observe(picture().as_bytes()),
            Observation::RecognizedTemplate
        ));
    }

    #[test]
    fn inherited_effects_crop_rotation_foreign_namespaces_and_missing_parts_are_unsupported() {
        let original = picture();
        for changed in [
            original.replace("</v:shapetype>", "<v:shadow on=\"t\"/></v:shapetype>"),
            original.replace("</v:shape>", "<v:shadow on=\"t\"/></v:shape>"),
            original.replace("o:title=\"\"", "o:title=\"\" croptop=\".5\""),
            original.replace("width:10pt", "width:10pt;rotation:1"),
            original.replace("width:10pt", "width:0pt"),
            original.replace("urn:schemas-microsoft-com:vml", "urn:foreign"),
            original.replace("sum @0 1 0", "sum @0 2 0"),
            original.replace("<x:SizeWithCells/>", ""),
            original.replace("<x:SizeWithCells/>", "<x:SizeWithCells/><x:SizeWithCells/>"),
        ] {
            assert_ne!(changed, original);
            assert!(matches!(
                observe(changed.as_bytes()),
                Observation::Unsupported { .. }
            ));
        }
    }
}
