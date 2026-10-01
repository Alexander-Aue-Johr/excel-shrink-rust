// Included by excel_automation.rs. Borrowed XML views; no worksheet DOM or
// decoding of cell text. Value bounds already come from value_views.rs.
#[derive(Debug, Clone, Default, Serialize)]
struct DrawingCapturePlan {
    max_row: u32,
    max_column: u32,
    points: Vec<DrawingCapturePoint>,
    has_unmodeled_extents: bool,
}

#[derive(Debug, Clone, Serialize)]
struct DrawingCapturePoint {
    row: u32, column: u32,
    right: f64, bottom: f64,
}

fn drawing_number(bytes: &[u8]) -> anyhow::Result<f64> {
    let n: f64 = std::str::from_utf8(bytes)?.trim().parse()?;
    anyhow::ensure!(n.is_finite() && n >= 0.0, "invalid drawing coordinate");
    Ok(n / 12700.0) // EMU -> points
}

fn drawing_marker(bytes: &[u8]) -> anyhow::Result<DrawingCapturePoint> {
    use super::xml_views::{Reader, Kind, decimal};
    let mut reader = Reader::<128>::new(bytes)?;
    let (mut row, mut column) = (None, None);
    let (mut right, mut bottom) = (0.0, 0.0);
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End { continue; }
        let body = reader.element(tag)?.body;
        match tag.local() {
            b"row" => row = Some(decimal(body)?.checked_add(1).ok_or_else(|| anyhow::anyhow!("row overflow"))?),
            b"col" => column = Some(decimal(body)?.checked_add(1).ok_or_else(|| anyhow::anyhow!("column overflow"))?),
            b"rowOff" => bottom = drawing_number(body)?,
            b"colOff" => right = drawing_number(body)?,
            _ => anyhow::bail!("unsupported drawing marker"),
        }
    }
    let row = row.ok_or_else(|| anyhow::anyhow!("drawing row missing"))?;
    let column = column.ok_or_else(|| anyhow::anyhow!("drawing column missing"))?;
    anyhow::ensure!(row <= 1048576 && column <= 16384, "drawing outside grid");
    Ok(DrawingCapturePoint { row, column, right, bottom })
}

fn parse_drawing_capture_plan(bytes: &[u8]) -> anyhow::Result<DrawingCapturePlan> {
    use super::xml_views::{Reader, Kind};
    let mut plan = DrawingCapturePlan::default();
    let mut reader = Reader::<128>::new(bytes)?;
    let mut root_seen = false;
    while let Some(tag) = reader.next()? {
        if tag.kind == Kind::End { continue; }
        if tag.depth == 0 {
            anyhow::ensure!(!root_seen && tag.local() == b"wsDr", "expected drawing root");
            root_seen = true; continue;
        }
        if tag.depth != 1 { continue; }
        let anchor = reader.element(tag)?;
        if !matches!(tag.local(), b"twoCellAnchor" | b"oneCellAnchor" | b"absoluteAnchor") {
            plan.has_unmodeled_extents = true; continue;
        }
        let mut children = Reader::<128>::new(anchor.body)?;
        let (mut from, mut to, mut pos, mut extent) = (None, None, None, None);
        while let Some(child) = children.next()? {
            if child.kind == Kind::End { continue; }
            let element = children.element(child)?;
            match child.local() {
                b"from" => from = Some(drawing_marker(element.body)?),
                b"to" => to = Some(drawing_marker(element.body)?),
                b"pos" => pos = Some(DrawingCapturePoint { row: 0, column: 0,
                    right: drawing_number(child.attr(b"x")?.ok_or_else(|| anyhow::anyhow!("missing x"))?)?,
                    bottom: drawing_number(child.attr(b"y")?.ok_or_else(|| anyhow::anyhow!("missing y"))?)? }),
                b"ext" => extent = Some((
                    drawing_number(child.attr(b"cx")?.ok_or_else(|| anyhow::anyhow!("missing cx"))?)?,
                    drawing_number(child.attr(b"cy")?.ok_or_else(|| anyhow::anyhow!("missing cy"))?)?)),
                _ => {
                    // Record effects/rotation for diagnostics; capture uses XML anchors.
                    // No COM shape enumeration or extra effect envelope is requested.
                    let mut shape = Reader::<128>::new(element.body)?;
                    while let Some(t) = shape.next()? {
                        if t.attr(b"rot")?.is_some() || (t.local() == b"ln" && t.attr(b"w")?.is_some()) || matches!(t.local(), b"effectLst" | b"effectDag" | b"AlternateContent") {
                            plan.has_unmodeled_extents = true;
                        }
                    }
                }
            }
        }
        if let Some(mode) = tag.attr(b"editAs")? { if mode != b"twoCell" { plan.has_unmodeled_extents = true; } }
        let point = if tag.local() == b"twoCellAnchor" {
            let a = from.ok_or_else(|| anyhow::anyhow!("missing from marker"))?;
            plan.max_row = plan.max_row.max(a.row); plan.max_column = plan.max_column.max(a.column);
            plan.points.push(a);
            to.ok_or_else(|| anyhow::anyhow!("missing to marker"))?
        } else {
            let mut p = if tag.local() == b"oneCellAnchor" { from } else { pos }
                .ok_or_else(|| anyhow::anyhow!("missing drawing origin"))?;
            let (w, h) = extent.ok_or_else(|| anyhow::anyhow!("missing drawing extent"))?;
            p.right += w; p.bottom += h; p
        };
        plan.max_row = plan.max_row.max(point.row); plan.max_column = plan.max_column.max(point.column);
        plan.points.push(point);
    }
    anyhow::ensure!(root_seen, "missing drawing root");
    Ok(plan)
}

fn workbook_drawing_capture_plans(path: &Path) -> anyhow::Result<BTreeMap<usize, DrawingCapturePlan>> {
    let _phase = visual_diagnostics::phase("screenshot.drawing_bounds", path.display().to_string());
    let order = workbook_visual_sheet_order(path)?;
    let mut zip = ZipArchive::new(File::open(path)?)?;
    let mut plans = BTreeMap::new();
    for (index, sheet) in order.iter().enumerate() {
        if !sheet.starts_with("xl/worksheets/") { continue; }
        let (dir, file) = sheet.rsplit_once('/').ok_or_else(|| anyhow::anyhow!("invalid sheet path"))?;
        let rels_name = format!("{dir}/_rels/{file}.rels");
        let mut data = Vec::new();
        match zip.by_name(&rels_name) {
            Ok(mut entry) => { entry.read_to_end(&mut data)?; },
            Err(zip::result::ZipError::FileNotFound) => continue,
            Err(e) => return Err(e.into()),
        }
        let mut plan = DrawingCapturePlan::default();
        for rel in visual_relationships(&data)? {
            let kind = rel.kind.rsplit('/').next().unwrap_or("");
            if matches!(kind, "vmlDrawing" | "oleObject" | "control") { plan.has_unmodeled_extents = true; }
            if kind != "drawing" { continue; }
            anyhow::ensure!(!rel.external, "external drawing cannot be bounded");
            let part = visual_resolve_target(sheet, &rel.target)?;
            let _read = visual_diagnostics::phase("screenshot.drawing_xml", part.clone());
            let mut bytes = Vec::new(); zip.by_name(&part)?.read_to_end(&mut bytes)?;
            let drawing = parse_drawing_capture_plan(&bytes)?;
            plan.max_row = plan.max_row.max(drawing.max_row);
            plan.max_column = plan.max_column.max(drawing.max_column);
            plan.points.extend(drawing.points); plan.has_unmodeled_extents |= drawing.has_unmodeled_extents;
        }
        plans.insert(index + 1, plan);
    }
    Ok(plans)
}

fn prepare_visual_capture_limits(original: &Path, shrunk: &Path, input: &[WorksheetCaptureRange])
    -> anyhow::Result<(Vec<WorksheetCaptureRange>, serde_json::Value)> {
    let a = workbook_drawing_capture_plans(original)?;
    let b = workbook_drawing_capture_plans(shrunk)?;
    let original_columns = super::xlsx_value_compare::workbook_capture_columns(original)?;
    let shrunk_columns = super::xlsx_value_compare::workbook_capture_columns(shrunk)?;
    let mut ranges = input.to_vec();
    let mut json = Vec::new();
    for range in &mut ranges {
        let mut points = Vec::new(); let mut unmodeled = false;
        for plan in [a.get(&range.sheet_index), b.get(&range.sheet_index)].into_iter().flatten() {
            if plan.max_row > range.max_row { range.max_row = plan.max_row; range.max_row_source = Some("drawing".into()); }
            if plan.max_column > range.max_column { range.max_column = plan.max_column; range.max_column_source = Some("drawing".into()); }
            points.extend(plan.points.clone()); unmodeled |= plan.has_unmodeled_extents;
        }
        let mut n = range.max_column; let mut col = String::new();
        while n > 0 { n -= 1; col.insert(0, (b'A' + (n % 26) as u8) as char); n /= 26; }
        range.max_cell = format!("{col}{}", range.max_row);
        let mut value = serde_json::to_value(&*range)?;
        value["drawing_points"] = serde_json::to_value(points)?;
        value["drawing_unmodeled_extents"] = serde_json::json!(unmodeled);
        // Chart sheets have no worksheet-column plan and use their own capture path.
        if let (Some(left), Some(right)) = (original_columns.get(&range.sheet_index), shrunk_columns.get(&range.sheet_index)) {
            value["original_columns"] = serde_json::to_value(left)?;
            value["shrunk_columns"] = serde_json::to_value(right)?;
            let widths = OverflowColumnPair::new(left, right)?;
            value["overflow_first_column"] = serde_json::json!(widths.end_column(range.max_column, 1));
        }

        json.push(value);
    }
    Ok((ranges, serde_json::Value::Array(json)))
}

#[cfg(test)]
mod drawing_bounds_tests {
    use super::*;
    #[test]
    fn prefixed_two_cell_extends_both_axes() {
        let p = parse_drawing_capture_plan(br#"<d:wsDr><d:twoCellAnchor><d:from><d:col>1</d:col><d:row>2</d:row></d:from><d:to><d:col>9</d:col><d:colOff>12700</d:colOff><d:row>39</d:row><d:rowOff>25400</d:rowOff></d:to><d:pic/></d:twoCellAnchor></d:wsDr>"#).unwrap();
        assert_eq!((p.max_row, p.max_column), (40, 10));
        assert_eq!((p.points[1].right, p.points[1].bottom), (1.0, 2.0));
    }
    #[test]
    fn absolute_extents_and_invalid_xml() {
        let p = parse_drawing_capture_plan(br#"<wsDr><absoluteAnchor><pos x="12700" y="25400"/><ext cx="38100" cy="50800"/></absoluteAnchor></wsDr>"#).unwrap();
        assert_eq!((p.points[0].right, p.points[0].bottom), (4.0, 6.0));
        assert!(parse_drawing_capture_plan(b"<wsDr><oneCellAnchor/></wsDr>").is_err());
        assert!(parse_drawing_capture_plan(b"<wrongRoot/>").is_err());
    }
    #[test]
    fn one_cell_extent_keeps_origin_and_marks_unmodeled_effects() {
        let p = parse_drawing_capture_plan(br#"<wsDr><oneCellAnchor><from><col>4</col><row>7</row><colOff>12700</colOff></from><ext cx="25400" cy="38100"/><sp><spPr><effectLst/></spPr></sp></oneCellAnchor></wsDr>"#).unwrap();
        assert_eq!((p.max_row, p.max_column), (8, 5));
        assert_eq!((p.points[0].right, p.points[0].bottom), (3.0, 3.0));
        assert!(p.has_unmodeled_extents);
    }
}
