//! Prove supported chart inputs unchanged; an incomplete dependency graph requires images.
use super::{
    charts::{Chart, ChartKind, Inventory},
    conditional_formatting::{self, cell_address},
    content_bounds::Rectangle,
    telemetry::LaneId,
    xml_changes::Snapshot,
};
use crate::regression::{
    office_plan::{decode_attr, formula_worksheet_parts},
    xml_views::{Kind, Reader},
};
use anyhow::{ensure, Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::Path,
};
use zip::ZipArchive;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(in crate::regression) enum Mode {
    Conservative,
    #[default]
    Deterministic,
}

#[derive(Serialize)]
pub(super) struct Decision {
    pub chart: Chart,
    pub unchanged: bool,
    pub reason: String,
    pub references: usize,
}

#[derive(Serialize)]
pub(super) struct Check {
    pub left: LaneId,
    pub right: LaneId,
    pub mode: Mode,
    pub decisions: Vec<Decision>,
}

struct Package {
    shared: [u8; 32],
    charts: BTreeMap<(String, String), String>,
    parts: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, PartialEq, Eq)]
struct Reference {
    sheet: String,
    area: Rectangle,
}

/// The graph is deliberately small: ordinary embedded charts with same-workbook A1 sources.
/// Static related parts must match byte-for-byte; unknown package features fail closed.
impl Package {
    fn read(path: &Path) -> Result<Self> {
        let mut zip = ZipArchive::new(File::open(path)?)?;
        let worksheets = formula_worksheet_parts(&mut zip)?;
        let mut shared = Sha256::new();
        let mut parts = BTreeMap::new();
        let mut names: Vec<_> = zip.file_names().map(str::to_owned).collect();
        names.sort();
        for name in names {
            if name.ends_with('/')
                || name.starts_with("docProps/")
                || name == "xl/calcChain.xml"
                || name == "xl/sharedStrings.xml"
                || name == "xl/styles.xml"
                || name.starts_with("xl/worksheets/") && name.ends_with(".xml")
            {
                continue;
            }
            ensure!(
                name == "[Content_Types].xml"
                    || name == "_rels/.rels"
                    || name == "xl/workbook.xml"
                    || name == "xl/_rels/workbook.xml.rels"
                    || [
                        "xl/charts/",
                        "xl/drawings/",
                        "xl/theme/",
                        "xl/media/",
                        "xl/worksheets/_rels/",
                        "xl/printerSettings/"
                    ]
                    .iter()
                    .any(|prefix| name.starts_with(prefix)),
                "Unsupported chart package dependency: {name}"
            );
            let mut bytes = Vec::new();
            zip.by_name(&name)?.read_to_end(&mut bytes)?;
            if name.ends_with(".rels") {
                relationships(&bytes)?;
            }
            if name.starts_with("xl/charts/chart") && name.ends_with(".xml") {
                parts.insert(name, bytes);
                continue;
            }
            shared.update((name.len() as u64).to_le_bytes());
            shared.update(name.as_bytes());
            if name == "xl/workbook.xml" {
                shared.update(workbook_identity(&bytes)?);
            } else {
                shared.update(Sha256::digest(&bytes));
            }
        }
        let mut charts = BTreeMap::new();
        let mut ambiguous = BTreeSet::new();
        for (sheet, part) in worksheets {
            let Some(rels) = optional_part(&mut zip, &rels_part(&part))? else {
                continue;
            };
            for (_, (kind, target)) in relationships(&rels)? {
                if !kind.ends_with("/drawing") {
                    continue;
                }
                let drawing = resolve(&part, &target)?;
                let drawing_rels = relationships(&part_bytes(&mut zip, &rels_part(&drawing))?)?;
                for (name, id) in drawing_charts(&part_bytes(&mut zip, &drawing)?)? {
                    let (kind, target) = drawing_rels
                        .get(&id)
                        .context("Missing chart relationship")?;
                    ensure!(kind.ends_with("/chart"), "Unknown drawing reference");
                    let target = resolve(&drawing, target)?;
                    ensure!(parts.contains_key(&target), "Missing chart XML");
                    let key = (sheet.clone(), name);
                    if !ambiguous.contains(&key) && charts.insert(key.clone(), target).is_some() {
                        charts.remove(&key);
                        ambiguous.insert(key);
                    }
                }
            }
        }
        Ok(Self {
            shared: shared.finalize().into(),
            charts,
            parts,
        })
    }
}

fn part_bytes(zip: &mut ZipArchive<File>, name: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    zip.by_name(name)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn optional_part(zip: &mut ZipArchive<File>, name: &str) -> Result<Option<Vec<u8>>> {
    match zip.by_name(name) {
        Ok(mut part) => {
            let mut bytes = Vec::new();
            part.read_to_end(&mut bytes)?;
            Ok(Some(bytes))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn rels_part(owner: &str) -> String {
    let (directory, filename) = owner.rsplit_once('/').unwrap_or(("", owner));
    format!("{directory}/_rels/{filename}.rels")
}

fn resolve(owner: &str, target: &str) -> Result<String> {
    ensure!(
        !target.contains(['\\', ':', '#', '?', '%']),
        "Unsupported package target"
    );
    let base = if target.starts_with('/') {
        ""
    } else {
        owner.rsplit_once('/').map_or("", |p| p.0)
    };
    let mut parts = Vec::new();
    for part in base.split('/').chain(target.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                ensure!(parts.pop().is_some(), "Target escapes package");
            }
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

fn relationships(bytes: &[u8]) -> Result<BTreeMap<String, (String, String)>> {
    let mut result = BTreeMap::new();
    let mut reader = Reader::<128>::new(bytes)?;
    while let Some(tag) = reader.next()? {
        if tag.depth == 0 && tag.kind != Kind::End {
            ensure!(
                tag.name == b"Relationships"
                    && tag.attr(b"xmlns")?
                        == Some(b"http://schemas.openxmlformats.org/package/2006/relationships"),
                "Unknown relationship namespace"
            );
        }
        if tag.local() != b"Relationship" || tag.kind == Kind::End {
            continue;
        }
        ensure!(
            tag.attr(b"TargetMode")?.is_none(),
            "External chart/package relationship"
        );
        let attr = |key| decode_attr(tag.attr(key)?.context("Incomplete relationship")?);
        ensure!(
            result
                .insert(attr(b"Id")?, (attr(b"Type")?, attr(b"Target")?))
                .is_none(),
            "Duplicate relationship ID"
        );
    }
    Ok(result)
}

/// Proven editing UUIDs and staging paths are not chart inputs. Keep all other workbook bytes.
fn workbook_identity(bytes: &[u8]) -> Result<[u8; 32]> {
    let mut reader = Reader::<128>::new(bytes)?;
    let mut metadata = super::static_workbook_metadata::WorkbookMetadata::default();
    let mut hash = Sha256::new();
    let mut copied = 0;
    while let Some(tag) = reader.next()? {
        let supported = metadata.accepts(tag)?;
        if matches!(tag.name, b"xr:revisionPtr" | b"x15ac:absPath") && supported && tag.kind == Kind::Empty {
            hash.update(&bytes[copied..tag.start]);
            copied = tag.end;
        }
    }
    hash.update(&bytes[copied..]);
    Ok(hash.finalize().into())
}

fn drawing_charts(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let mut reader = Reader::<128>::new(bytes)?;
    let mut frame = None;
    let mut name = None;
    let mut charts = Vec::new();
    while let Some(tag) = reader.next()? {
        if tag.depth == 0 && tag.kind != Kind::End {
            ensure!(
                tag.name == b"xdr:wsDr"
                    && tag.attr(b"xmlns:xdr")?
                        == Some(
                            b"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"
                        ),
                "Unknown drawing namespace"
            );
        }
        if tag.local() == b"graphicFrame" {
            if tag.kind == Kind::Start {
                frame = Some(tag.depth);
                name = None;
            } else if tag.kind == Kind::End {
                frame = None;
            }
        }
        if frame.is_none() || tag.kind == Kind::End {
            continue;
        }
        if tag.local() == b"cNvPr" {
            name = Some(decode_attr(tag.attr(b"name")?.context("Unnamed chart")?)?);
        }
        if tag.name == b"c:chart" {
            charts.push((
                name.clone().context("Missing chart object name")?,
                decode_attr(tag.attr(b"r:id")?.context("Missing chart relationship")?)?,
            ));
        }
    }
    Ok(charts)
}

/// No formula evaluation: accept only one explicit same-workbook sheet and finite A1 rectangle.
fn reference(text: &str) -> Option<Reference> {
    let (sheet, range) = text.split_once('!')?;
    if range.contains('!') || sheet.contains(['[', ']', ':']) {
        return None;
    }
    let sheet = if sheet.starts_with('\'') && sheet.ends_with('\'') {
        let inner = &sheet[1..sheet.len() - 1];
        let decoded = inner.replace("''", "");
        if decoded.contains('\'') {
            return None;
        }
        inner.replace("''", "'")
    } else {
        if !sheet
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return None;
        }
        sheet.to_owned()
    };
    if sheet.is_empty() {
        return None;
    }
    let (first, last) = range.split_once(':').unwrap_or((range, range));
    if !a1_address(first) || !a1_address(last) {
        return None;
    }
    let (top, left) = cell_address(first)?;
    let (bottom, right) = cell_address(last)?;
    (top > 0 && left > 0 && top <= bottom && left <= right).then_some(Reference {
        sheet,
        area: Rectangle {
            top,
            left,
            bottom,
            right,
        },
    })
}

fn a1_address(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = usize::from(bytes.first() == Some(&b'$'));
    let letters = index;
    while bytes.get(index).is_some_and(u8::is_ascii_uppercase) {
        index += 1;
    }
    if index == letters {
        return false;
    }
    index += usize::from(bytes.get(index) == Some(&b'$'));
    let digits = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    index > digits && index == bytes.len()
}

fn chart_references(bytes: &[u8]) -> Result<Vec<Reference>> {
    let mut reader = Reader::<128>::new(bytes)?;
    let mut formula = None;
    let mut refs = Vec::new();
    let mut extension = None;
    let mut unique_namespace = false;
    let mut display_namespace = false;
    let mut root_unique_namespace = false;
    let mut root_display_namespace = false;
    while let Some(tag) = reader.next()? {
        if tag.depth == 0 && tag.kind == Kind::Start {
            ensure!(
                tag.name == b"c:chartSpace"
                    && tag.attr(b"xmlns:c")?
                        == Some(b"http://schemas.openxmlformats.org/drawingml/2006/chart"),
                "Unknown chart namespace"
            );
            unique_namespace = tag.attr(b"xmlns:c16")?
                == Some(b"http://schemas.microsoft.com/office/drawing/2014/chart");
            display_namespace = tag.attr(b"xmlns:c16r3")?
                == Some(b"http://schemas.microsoft.com/office/drawing/2017/03/chart");
            root_unique_namespace = unique_namespace;
            root_display_namespace = display_namespace;
        }
        if let Some(namespace) = tag.attr(b"xmlns:c")? {
            ensure!(
                namespace == b"http://schemas.openxmlformats.org/drawingml/2006/chart",
                "Rebound chart namespace"
            );
        }
        for (prefix, expected) in [
            (
                b"xmlns:c16".as_slice(),
                b"http://schemas.microsoft.com/office/drawing/2014/chart".as_slice(),
            ),
            (
                b"xmlns:c16r3".as_slice(),
                b"http://schemas.microsoft.com/office/drawing/2017/03/chart".as_slice(),
            ),
        ] {
            if let Some(namespace) = tag.attr(prefix)? {
                ensure!(namespace == expected, "Unknown chart extension namespace");
            }
        }
        ensure!(
            !matches!(
                tag.local(),
                b"externalData" | b"pivotSource" | b"userShapes" | b"fld"
            ),
            "Dynamic or external chart dependency"
        );
        if tag.name == b"c:ext" && tag.kind != Kind::End {
            ensure!(
                matches!(
                    tag.attr(b"uri")?,
                    Some(
                        b"{C3380CC4-5D6E-409C-BE32-E72D297353CC}"
                            | b"{56B9EC1D-385E-4148-901F-78D8002777C0}"
                    )
                ),
                "Unknown chart extension"
            );
            extension = (tag.kind == Kind::Start).then_some(tag.depth);
            unique_namespace = root_unique_namespace || tag.attr(b"xmlns:c16")?.is_some();
            display_namespace = root_display_namespace || tag.attr(b"xmlns:c16r3")?.is_some();
        } else if let Some(depth) = extension {
            if tag.name == b"c:ext" && tag.kind == Kind::End && tag.depth == depth {
                extension = None;
            } else {
                let known = match tag.name {
                    b"c16:uniqueId" => {
                        tag.kind == Kind::Empty
                            && (unique_namespace || tag.attr(b"xmlns:c16")?.is_some())
                    }
                    b"c16r3:dataDisplayOptions16" => {
                        display_namespace |= tag.attr(b"xmlns:c16r3")?.is_some();
                        display_namespace
                    }
                    b"c16r3:dispNaAsBlank" => display_namespace && tag.kind == Kind::Empty,
                    _ => false,
                };
                ensure!(known, "Unknown chart extension payload");
                for attr in crate::regression::xml_views::Attributes::new(tag.attrs) {
                    ensure!(
                        matches!(attr?.0, b"val" | b"xmlns:c16" | b"xmlns:c16r3"),
                        "Unknown chart extension attribute"
                    );
                }
            }
        }
        if tag.local() == b"f" {
            ensure!(tag.name == b"c:f", "Unknown formula namespace");
            match tag.kind {
                Kind::Start => formula = Some(tag.end),
                Kind::End => {
                    let start = formula.take().context("Unmatched chart formula")?;
                    let text = decode_attr(&bytes[start..tag.start])?;
                    refs.push(reference(&text).context("Unsupported chart reference")?);
                }
                Kind::Empty => anyhow::bail!("Empty chart reference"),
            }
        }
    }
    ensure!(
        !refs.is_empty(),
        "No supported cell dependencies; literal-only charts retain captures"
    );
    Ok(refs)
}

/// Prove cached chart data against actual rebuilt cell fingerprints, source layout and CF inputs.
pub(super) fn select(
    left_path: &Path,
    right_path: &Path,
    left: &Snapshot,
    right: &Snapshot,
    cf: &[conditional_formatting::Decision],
    inventory: &[Inventory],
    partner: &[Inventory],
    mode: Mode,
) -> Vec<Decision> {
    if inventory.is_empty() {
        return Vec::new();
    }
    let packages = if mode == Mode::Deterministic {
        Package::read(left_path).and_then(|a| {
            let b = Package::read(right_path)?;
            ensure!(
                a.shared == b.shared
                    && left.context == right.context
                    && left.cf_context == right.cf_context,
                "Changed package, theme, format, string or workbook dependencies"
            );
            Ok((a, b))
        })
    } else {
        Err(anyhow::anyhow!("Conservative chart capture requested"))
    };
    let changed = packages
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error:#}"))
        .and_then(|(a, _)| referenced_changes(a, left, right));
    inventory.iter().map(|entry| {
        let proof = (|| -> Result<usize> {
            let (a, b) = packages.as_ref().map_err(|error| anyhow::anyhow!("{error:#}"))?;
            let changed = changed.as_ref().map_err(|error| anyhow::anyhow!("{error:#}"))?;
            ensure!(entry.chart.kind == ChartKind::Embedded, "Chart sheet retains capture");
            let other = partner.iter().find(|p| p.chart == entry.chart).context("Missing partner chart")?;
            entry.valid_geometry()?;
            other.valid_geometry()?;
            ensure!(entry.geometry.is_some() && entry.geometry == other.geometry,
                "Missing or changed live chart geometry");
            let key = (entry.chart.sheet.clone(), entry.chart.name.clone());
            let part = a.charts.get(&key).context("Chart XML mapping unavailable")?;
            ensure!(b.charts.get(&key) == Some(part), "Changed chart mapping");
            let xml = &a.parts[part];
            ensure!(b.parts.get(part) == Some(xml), "Changed chart XML or cached data");
            let references = chart_references(xml)?;
            for reference in &references {
                let matches: Vec<_> = left.sheets.keys().filter(|s| s.eq_ignore_ascii_case(&reference.sheet)).collect();
                ensure!(matches.len() == 1, "Unknown or ambiguous source sheet");
                let sheet = matches[0];
                let x = &left.sheets[sheet];
                let y = right.sheets.get(sheet).context("Missing source sheet")?;
                let rect = reference.area;
                ensure!(x.value_namespace_supported && y.value_namespace_supported,
                    "Unsupported worksheet value namespace");
                ensure!(x.defaults == y.defaults && x.cf_layout == y.cf_layout
                    && x.cf_columns.same_prefix(&y.cf_columns, rect.right),
                    "Changed source layout or inherited column formatting");
                ensure!(x.rows.range(rect.top..=rect.bottom).eq(y.rows.range(rect.top..=rect.bottom)),
                    "Changed source row visibility or formatting");
                ensure!(!cf.iter().any(|d| d.sheet == *sheet && d.outcome != "excluded"),
                    "Unresolved source conditional formatting");
                ensure!(!changed.get(sheet).is_some_and(|cells| cells.range((rect.top, 0)..=(rect.bottom, 16384))
                    .any(|&(_, column)| column >= rect.left && column <= rect.right)),
                    "Changed referenced cell value, formula or style");
            }
            Ok(references.len())
        })();
        match proof {
            Ok(references) => Decision { chart: entry.chart.clone(), unchanged: true,
                reason: "Unchanged chart XML/cache, referenced cells, source formatting, package dependencies and live geometry".into(), references },
            Err(error) => Decision { chart: entry.chart.clone(), unchanged: false,
                reason: format!("{error:#}"), references: 0 },
        }
    }).collect()
}

/// Chart sources can lie outside the worksheet capture envelope. Scan complete cell streams,
/// retaining only changes inside known chart references; never reuse the clipped cell selection.
fn referenced_changes(
    package: &Package,
    left: &Snapshot,
    right: &Snapshot,
) -> Result<BTreeMap<String, BTreeSet<(u32, u32)>>> {
    let mut areas = BTreeMap::<String, Vec<Rectangle>>::new();
    for bytes in package.parts.values() {
        // Unsupported charts retain captures. Only supported references can establish exclusions.
        if let Ok(references) = chart_references(bytes) {
            for reference in references {
                let names: Vec<_> = left
                    .sheets
                    .keys()
                    .filter(|s| s.eq_ignore_ascii_case(&reference.sheet))
                    .collect();
                if names.len() == 1 {
                    areas
                        .entry(names[0].clone())
                        .or_default()
                        .push(reference.area);
                }
            }
        }
    }
    let mut changed = BTreeMap::new();
    for (sheet, areas) in areas {
        let mut coordinates = BTreeSet::new();
        let partner = right
            .sheets
            .get(&sheet)
            .context("Missing referenced worksheet")?;
        left.sheets[&sheet]
            .cells
            .changes(&partner.cells, |change| {
                let coordinate = change.coordinate();
                let row = coordinate.row;
                let column = u32::from(coordinate.column);
                if areas.iter().any(|a| {
                    row >= a.top && row <= a.bottom && column >= a.left && column <= a.right
                }) {
                    coordinates.insert((row, column));
                }
                Ok(())
            })?;
        changed.insert(sheet, coordinates);
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, path::PathBuf};

    const CHART_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
    const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

    fn chart_xml(formula: &str, value: &str) -> String {
        format!(
            r#"<c:chartSpace xmlns:c="{CHART_NS}"><c:chart><c:plotArea><c:lineChart><c:ser><c:val><c:numRef><c:f>{formula}</c:f><c:numCache><c:pt idx="0"><c:v>{value}</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:lineChart></c:plotArea></c:chart></c:chartSpace>"#
        )
    }

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn fixture(chart: &str, row: &str, theme: &str) -> Result<Fixture> {
        let path = std::env::temp_dir().join(format!("chart-proof-{}.xlsx", uuid::Uuid::new_v4()));
        let mut zip = zip::ZipWriter::new(File::create(&path)?);
        let parts = [
            ("xl/workbook.xml", r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#.to_owned()),
            ("xl/_rels/workbook.xml.rels", format!(r#"<Relationships xmlns="{REL_NS}"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#)),
            ("xl/worksheets/sheet1.xml", format!(r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{row}</sheetData></worksheet>"#)),
            ("xl/worksheets/_rels/sheet1.xml.rels", format!(r#"<Relationships xmlns="{REL_NS}"><Relationship Id="rId1" Type="x/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#)),
            ("xl/drawings/drawing1.xml", r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><xdr:twoCellAnchor><xdr:graphicFrame><xdr:cNvPr name="Chart 1"/><c:chart r:id="rId1"/></xdr:graphicFrame></xdr:twoCellAnchor></xdr:wsDr>"#.to_owned()),
            ("xl/drawings/_rels/drawing1.xml.rels", format!(r#"<Relationships xmlns="{REL_NS}"><Relationship Id="rId1" Type="x/chart" Target="../charts/chart1.xml"/></Relationships>"#)),
            ("xl/charts/chart1.xml", chart.to_owned()),
            ("xl/theme/theme1.xml", theme.to_owned()),
        ];
        for (name, text) in parts {
            zip.start_file(name, zip::write::FileOptions::default())?;
            zip.write_all(text.as_bytes())?;
        }
        zip.finish()?;
        Ok(Fixture(path))
    }

    fn inventory() -> Vec<Inventory> {
        vec![Inventory {
            chart: Chart {
                sheet: "Data".into(),
                name: "Chart 1".into(),
                index: Some(0),
                kind: ChartKind::Embedded,
            },
            geometry: Some(serde_json::json!({"top":0,"left":0,"width":400,"height":200})),
        }]
    }

    fn decision(a: &Fixture, b: &Fixture) -> Result<Decision> {
        let x = Snapshot::read(&a.0)?;
        let y = Snapshot::read(&b.0)?;
        Ok(select(
            &a.0,
            &b.0,
            &x,
            &y,
            &[],
            &inventory(),
            &inventory(),
            Mode::Deterministic,
        )
        .remove(0))
    }

    #[test]
    fn same_cache_requires_actual_unchanged_cells_and_static_dependencies() -> Result<()> {
        let chart = chart_xml("Data!$A$1:$A$1", "1");
        let row = r#"<row r="1"><c r="A1"><v>1</v></c></row>"#;
        let a = fixture(&chart, row, "same theme")?;
        assert!(decision(&a, &a)?.unchanged);
        let changed_value = fixture(&chart, &row.replace(">1</v>", ">2</v>"), "same theme")?;
        assert!(!decision(&a, &changed_value)?.unchanged);
        let changed_style = fixture(
            &chart,
            &row.replace("r=\"A1\"", "r=\"A1\" s=\"4\""),
            "same theme",
        )?;
        assert!(!decision(&a, &changed_style)?.unchanged);
        let changed_hidden = fixture(
            &chart,
            &row.replace("r=\"1\"", "r=\"1\" hidden=\"1\""),
            "same theme",
        )?;
        assert!(!decision(&a, &changed_hidden)?.unchanged);
        let changed_theme = fixture(&chart, row, "different theme")?;
        assert!(!decision(&a, &changed_theme)?.unchanged);
        let changed_cache = fixture(&chart_xml("Data!$A$1:$A$1", "2"), row, "same theme")?;
        assert!(!decision(&a, &changed_cache)?.unchanged);
        Ok(())
    }

    #[test]
    fn missing_geometry_partner_and_conservative_mode_retain_images() -> Result<()> {
        let a = fixture(
            &chart_xml("Data!A1", "1"),
            r#"<row r="1"><c r="A1"><v>1</v></c></row>"#,
            "theme",
        )?;
        let x = Snapshot::read(&a.0)?;
        assert!(
            !select(
                &a.0,
                &a.0,
                &x,
                &x,
                &[],
                &inventory(),
                &inventory(),
                Mode::Conservative
            )[0]
            .unchanged
        );
        assert!(
            !select(
                &a.0,
                &a.0,
                &x,
                &x,
                &[],
                &inventory(),
                &[],
                Mode::Deterministic
            )[0]
            .unchanged
        );
        let mut other = inventory();
        other[0].geometry = None;
        assert!(
            !select(
                &a.0,
                &a.0,
                &x,
                &x,
                &[],
                &inventory(),
                &other,
                Mode::Deterministic
            )[0]
            .unchanged
        );
        other[0].geometry = Some(serde_json::json!({}));
        assert!(!select(&a.0, &a.0, &x, &x, &[], &other, &other, Mode::Deterministic)[0].unchanged);
        other[0].geometry = Some(serde_json::json!({"top":0,"left":0,"width":401,"height":200}));
        assert!(
            !select(
                &a.0,
                &a.0,
                &x,
                &x,
                &[],
                &inventory(),
                &other,
                Mode::Deterministic
            )[0]
            .unchanged
        );
        Ok(())
    }

    #[test]
    fn chart_sources_outside_capture_content_keep_removed_blank_style_dependencies() -> Result<()> {
        let chart = chart_xml("Data!Z1000", "0");
        let a = fixture(
            &chart,
            r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="1000"><c r="Z1000"/></row>"#,
            "theme",
        )?;
        let b = fixture(
            &chart,
            r#"<row r="1"><c r="A1"><v>1</v></c></row>"#,
            "theme",
        )?;
        let x = Snapshot::read(&a.0)?;
        let y = Snapshot::read(&b.0)?;
        assert!(x.content_changes(&y)?.0.is_empty());
        assert!(!decision(&a, &b)?.unchanged);
        Ok(())
    }

    #[test]
    fn references_and_unknown_extensions_fail_closed() -> Result<()> {
        assert_eq!(
            reference("'O''Brien Data'!$B$2:$C$9").unwrap().sheet,
            "O'Brien Data"
        );
        for unsupported in [
            "DefinedName",
            "[other.xlsx]Data!A1",
            "Data:Last!A1",
            "Data!A:A",
            "Data!OFFSET(A1,1,0)",
            "Data!A0",
            "Data!A1,B2",
        ] {
            assert!(reference(unsupported).is_none(), "{unsupported}");
        }
        assert!(chart_references(chart_xml("Data!A1", "1").as_bytes()).is_ok());
        let unknown = chart_xml("Data!A1", "1").replace("</c:chartSpace>", r#"<c:extLst><c:ext uri="unknown"><x:ref xmlns:x="unknown">Data!A2</x:ref></c:ext></c:extLst></c:chartSpace>"#);
        assert!(chart_references(unknown.as_bytes()).is_err());
        let known = chart_xml("Data!A1", "1").replace("</c:chartSpace>",
            r#"<c:extLst><c:ext uri="{C3380CC4-5D6E-409C-BE32-E72D297353CC}" xmlns:c16="http://schemas.microsoft.com/office/drawing/2014/chart"><c16:uniqueId val="id"/></c:ext></c:extLst></c:chartSpace>"#);
        assert!(chart_references(known.as_bytes()).is_ok());
        assert!(chart_references(
            known
                .replace("c16:uniqueId", "c16:formulaReference")
                .as_bytes()
        )
        .is_err());
        assert!(chart_references(
            known
                .replace("drawing/2014/chart", "drawing/unknown/chart")
                .as_bytes()
        )
        .is_err());
        assert!(chart_references(chart_xml("DefinedName", "1").as_bytes()).is_err());
        Ok(())
    }

    #[test]
    fn duplicate_names_only_disqualify_ambiguous_chart_objects() -> Result<()> {
        let a = fixture(
            &chart_xml("Data!A1", "1"),
            r#"<row r="1"><c r="A1"><v>1</v></c></row>"#,
            "theme",
        )?;
        let mut input = ZipArchive::new(File::open(&a.0)?)?;
        let b = Fixture(
            std::env::temp_dir().join(format!("chart-duplicates-{}.xlsx", uuid::Uuid::new_v4())),
        );
        let mut output = zip::ZipWriter::new(File::create(&b.0)?);
        for index in 0..input.len() {
            let mut part = input.by_index(index)?;
            let name = part.name().to_owned();
            let mut bytes = Vec::new();
            part.read_to_end(&mut bytes)?;
            if name == "xl/drawings/drawing1.xml" {
                let text = String::from_utf8(bytes)?;
                let start = text.find("<xdr:twoCellAnchor>").unwrap();
                let end = text.find("</xdr:twoCellAnchor>").unwrap() + "</xdr:twoCellAnchor>".len();
                let anchor = &text[start..end];
                bytes = text
                    .replace(
                        "</xdr:wsDr>",
                        &format!(
                            "{anchor}{}</xdr:wsDr>",
                            anchor.replace("Chart 1", "Chart 2")
                        ),
                    )
                    .into_bytes();
            }
            output.start_file(name, zip::write::FileOptions::default())?;
            output.write_all(&bytes)?;
        }
        output.finish()?;
        let package = Package::read(&b.0)?;
        assert!(!package
            .charts
            .contains_key(&("Data".into(), "Chart 1".into())));
        assert!(package
            .charts
            .contains_key(&("Data".into(), "Chart 2".into())));
        Ok(())
    }

    /// Exercise the production dependency gate against retained rebuilt files and measured inventories.
    #[test]
    #[ignore = "Requires XML_DIAG_LEFT, XML_DIAG_RIGHT, CHART_DIAG_INVENTORY and XML_DIAG_OUTPUT"]
    fn inspect_retained_chart_dependencies() -> Result<()> {
        let left = std::path::PathBuf::from(std::env::var("XML_DIAG_LEFT")?);
        let right = std::path::PathBuf::from(std::env::var("XML_DIAG_RIGHT")?);
        let a = Snapshot::read(&left)?;
        let b = Snapshot::read(&right)?;
        let changes = a.content_changes(&b)?.0;
        let cf = conditional_formatting::select(&a, &b, &changes, &mut BTreeSet::new())?;
        let inventory: BTreeMap<String, Vec<Inventory>> =
            serde_json::from_slice(&std::fs::read(std::env::var("CHART_DIAG_INVENTORY")?)?)?;
        let decisions = select(
            &left,
            &right,
            &a,
            &b,
            &cf,
            &inventory["left"],
            &inventory["right"],
            Mode::Deterministic,
        );
        let summary = serde_json::json!({"skipped":decisions.iter().filter(|d| d.unchanged).count(),
            "capture":decisions.iter().filter(|d| !d.unchanged).count(), "decisions":decisions});
        std::fs::write(
            std::env::var("XML_DIAG_OUTPUT")?,
            serde_json::to_vec_pretty(&summary)?,
        )?;
        eprintln!("{summary}");
        Ok(())
    }
}
