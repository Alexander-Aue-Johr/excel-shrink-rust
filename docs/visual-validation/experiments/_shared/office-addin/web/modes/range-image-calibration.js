/* global Excel, VVRenderDiagnostics */
'use strict';
(async function () {
  const D = VVRenderDiagnostics;
  let session;
  function unitFits(probes) {
    const xs = probes.filter((p) => p.success && p.measure_axis === 'x');
    const ys = probes.filter((p) => p.success && p.measure_axis === 'y');
    const singleCols = xs.filter((p) => p.xml_column_width !== undefined);
    const singleRows = ys.filter((p) => p.xml_row_height_points !== undefined);
    const candidates = [];
    // Candidate test for the documented width-in-file conversion. Not UI ColumnWidth.
    for (let mdw = 1; mdw <= 32; mdw++) {
      const residuals = singleCols.map((p) => {
        const predicted = Math.floor(((256 * p.xml_column_width + Math.floor(128 / mdw)) / 256) * mdw);
        return predicted - p.range_width_points * 96 / 72;
      });
      candidates.push({ maximum_digit_width_px_96dpi: mdw,
        rmse_px: residuals.length ? Math.sqrt(residuals.reduce((s, x) => s + x * x, 0) / residuals.length) : null,
        max_abs_residual_px: residuals.length ? Math.max(...residuals.map(Math.abs)) : null });
    }
    candidates.sort((a, b) => a.rmse_px - b.rmse_px);
    return {
      png_x_from_office_points: D.fit(xs.map((p) => [p.range_width_points, p.png_width])),
      png_y_from_office_points: D.fit(ys.map((p) => [p.range_height_points, p.png_height])),
      office_row_points_from_xml_ht: D.fit(singleRows.map((p) => [p.xml_row_height_points, p.range_height_points])),
      office_column_points_from_xml_width_linear_diagnostic: D.fit(singleCols.map((p) => [p.xml_column_width, p.range_width_points])),
      best_documented_xml_width_model_candidates: candidates.slice(0, 3),
      note: 'XML col.width is font-dependent, XML row.ht is points. The fitted PNG offset is separate from the scale. Font fallback, rounding and very small widths may cause residuals.'
    };
  }
  async function units(fixture) {
    const base = [];
    const widths = fixture.widths, heights = fixture.heights;
    await session.activate('Units');
    const font = await Excel.run(async (c) => {
      const f = c.workbook.worksheets.getItem('Units').getRange('A1').format.font;
      f.load('name,size'); await c.sync(); return { name: f.name, size_points: f.size };
    });
    session.analysis.requested_normal_font = { name: fixture.normal_font, size_points: fixture.normal_font_size_pt };
    session.analysis.api_cell_font = font;
    session.analysis.font_note = 'Font name read from the cell is not proof that Windows did not substitute a missing font.';
    for (let i = 0; i < widths.length; i++) {
      const c = D.columnName(i + 1);
      base.push(await session.probe({ sheet: 'Units', address: `${c}6:${c}6`, role: 'xml-column-width',
        measure_axis: 'x', xml_column_width: widths[i], xml_row_height_points: heights[5] }));
    }
    for (let i = 0; i < heights.length; i++) {
      base.push(await session.probe({ sheet: 'Units', address: `J${i + 1}:J${i + 1}`, role: 'xml-row-height',
        measure_axis: 'y', xml_row_height_points: heights[i], xml_column_width: widths[9] }));
    }
    for (const n of [1, 2, 4, 8, widths.length]) {
      base.push(await session.probe({ sheet: 'Units', address: `A6:${D.columnName(n)}6`, role: 'cumulative-width',
        measure_axis: 'x', xml_sum_column_width_units: widths.slice(0, n).reduce((a, b) => a + b, 0) }));
    }
    for (const n of [1, 2, 4, 8, heights.length]) {
      base.push(await session.probe({ sheet: 'Units', address: `J1:J${n}`, role: 'cumulative-height',
        measure_axis: 'y', xml_sum_height_points: heights.slice(0, n).reduce((a, b) => a + b, 0) }));
    }
    session.analysis.unit_conversion = unitFits(base);
    await session.emit('unit_conversion_summary', session.analysis.unit_conversion);
    // Same workbook, same XML geometry: isolate view zoom, grid lines, explicit borders.
    const viewComparisons = [];
    for (const sheet of ['Units', 'Zoom150', 'GridOn', 'Borders']) {
      await session.activate(sheet);
      for (const address of ['J6:J6', 'A1:M11']) {
        viewComparisons.push(await session.probe({ sheet, address, role: 'view-control',
          requested_xml_zoom: sheet === 'Zoom150' ? 150 : 100,
          requested_grid_lines: sheet === 'GridOn', requested_thin_borders: sheet === 'Borders' },
          { persist: address === 'A1:M11' }));
      }
    }
    session.analysis.view_controls = viewComparisons;
    const fractional = [];
    for (const sheet of ['Fractional', 'Defaults']) {
      await session.activate(sheet);
      for (const n of [1, 10, 100, 1000]) {
        fractional.push(await session.probe({ sheet, address: `A1:A${n}`, role: 'fractional-height-accumulation',
          explicit_row_ht: sheet === 'Fractional', explicit_column_width: sheet === 'Fractional',
          xml_sum_height_points: n * fixture.fractional_row_height_pt,
          xml_column_width: fixture.fractional_column_width_xml, rows_requested: n }));
      }
    }
    session.analysis.fractional_and_default_geometry = fractional;
    await session.activate('Hidden');
    session.analysis.hidden_geometry = await session.probe({ sheet: 'Hidden', address: 'A1:H8', role: 'hidden-row-column-control',
      xml_sum_height_points_before_hiding: 8 * 12.75,
      xml_sum_visible_height_points: 6 * 12.75, xml_visible_column_count: 6, xml_column_width: 10,
      hidden_rows: [3, 4], hidden_columns: [3, 4] }, { persist: true });
  }
  async function markerGeometry(sheet, markers, axis) {
    if (session.halted) throw new Error('Cannot query markers after an unresolved Office call');
    return Excel.run(async (c) => {
      const ws = c.workbook.worksheets.getItem(sheet);
      const requests = markers.map((m) => {
        const address = axis === 'y'
          ? `A${m.start_index}:D${m.end_index}`
          : `${D.columnName(m.start_index)}1:${D.columnName(m.end_index)}4`;
        const range = ws.getRange(address); range.load('top,left,height,width'); return { m, range };
      });
      await c.sync();
      return requests.map(({ m, range }) => ({ name: m.name, rgb: m.rgb, start_index: m.start_index, end_index: m.end_index,
        start_points: Number(axis === 'y' ? range.top : range.left),
        end_points: Number(axis === 'y' ? range.top + range.height : range.left + range.width) }));
    });
  }
  async function markers(fixture) {
    session.analysis.synthetic_controls = { normal_font: fixture.normal_font,
      row_height_xml_points: fixture.row_height_pt, column_width_xml_units: fixture.column_width_xml,
      no_merged_cells: true, no_drawings: true, grid_lines: false };
    await session.activate('Tall');
    const tallMarkers = await markerGeometry('Tall', fixture.tall_markers, 'y');
    const yBoundary = await D.plateauSearch(session, { sheet: 'Tall', axis: 'y', maxIndex: fixture.tall_rows,
      baseline: [1, 40, 100], address: (n) => `A1:D${n}`, controlAddress: 'A1:D40', neighborhoodRadius: 3 });
    session.analysis.synthetic_height_plateau = yBoundary;
    await session.checkpoint();
    if (session.halted) return;
    const tallResults = [];
    // Small marker control establishes that the fill detector actually sees the bands.
    const smallMarkers = tallMarkers.filter((m) => m.end_index <= 1600);
    const small = await session.probe({ sheet: 'Tall', address: 'A1:D1600', role: 'marker-detector-control' },
      { persist: true, markers: smallMarkers, markerAxis: 'y' });
    const detectorOk = small.marker_analysis && smallMarkers.every((m) => small.marker_analysis.markers.some((p) => p.name === m.name && p.count > 2));
    session.analysis.vertical_marker_control_ok = !!detectorOk;
    // Same height, 4x different width, therefore 4x different area.
    for (const col of ['A', 'D']) {
      const p = await session.probe({ sheet: 'Tall', address: `A1:${col}${fixture.tall_rows}`, role: 'tall-marker-evidence',
        fixed_last_column: col }, { persist: true, markers: tallMarkers, markerAxis: 'y' });
      tallResults.push({ probe_sequence: p.sequence,
        verdict: detectorOk ? D.markerVerdict(p, tallMarkers, 'y', yBoundary.calibration) : { conclusion: 'inconclusive-detector-control-failed' } });
    }
    session.analysis.vertical_marker_evidence = tallResults;
    // Medium-width control is inspected only as a PNG header, never decoded into a large bitmap.
    await session.probe({ sheet: 'Tall', address: `A1:P${fixture.tall_rows}`, role: 'height-cap-area-control-16-columns' });
    await session.activate('Wide');
    const wideMarkers = await markerGeometry('Wide', fixture.wide_markers, 'x');
    const xBoundary = await D.plateauSearch(session, { sheet: 'Wide', axis: 'x', maxIndex: fixture.wide_columns,
      baseline: [1, 8, 32], address: (n) => `A1:${D.columnName(n)}4`, controlAddress: 'A1:H4', neighborhoodRadius: 3 });
    session.analysis.synthetic_width_plateau = xBoundary;
    if (session.halted) return;
    const smallWideMarkers = wideMarkers.filter((m) => m.end_index <= 256);
    const smallWide = await session.probe({ sheet: 'Wide', address: 'A1:IV4', role: 'horizontal-marker-detector-control' },
      { persist: true, markers: smallWideMarkers, markerAxis: 'x' });
    const xDetectorOk = smallWide.marker_analysis && smallWideMarkers.every((m) => smallWide.marker_analysis.markers.some((p) => p.name === m.name && p.count > 2));
    const wide = await session.probe({ sheet: 'Wide', address: `A1:${D.columnName(fixture.wide_columns)}4`, role: 'wide-marker-evidence' },
      { persist: true, markers: wideMarkers, markerAxis: 'x' });
    session.analysis.horizontal_marker_control_ok = !!xDetectorOk;
    session.analysis.horizontal_marker_evidence = xDetectorOk ? D.markerVerdict(wide, wideMarkers, 'x', xBoundary.calibration) : { conclusion: 'inconclusive-detector-control-failed' };
    // Absolute-coordinate control: same short extent far down/right, beyond the suspected cap.
    await session.probe({ sheet: 'Tall', address: 'A3001:D3100', role: 'far-down-small-extent-control' }, { persist: true });
    await session.probe({ sheet: 'Wide', address: 'XA1:ZZ4', role: 'far-right-small-extent-control' }, { persist: true });
  }
  try {
    session = await D.open('range-image-calibration');
    if (!session) return;
    const fixture = (session.options.fixtures || []).find((f) => f.workbook === session.workbook);
    if (!fixture) throw new Error('No known synthetic fixture metadata for ' + session.workbook);
    session.analysis.fixture = fixture;
    if (fixture.kind === 'units') await units(fixture);
    else if (fixture.kind === 'markers') await markers(fixture);
    else if (fixture.kind === 'row-heights') {
      await new Promise((resolve, reject) => {
        const script = document.createElement('script');
        script.src = '/row-height-diagnostics.js?run=' + encodeURIComponent(session.config.diagnostic_run_id);
        script.onload = resolve; script.onerror = () => reject(new Error('Cannot load row-height diagnostics'));
        document.head.appendChild(script);
      });
      await window.VVRowHeightDiagnostics.run(session, fixture);
    }
    else throw new Error('Unknown fixture kind');
    session.analysis.render_error_count = session.probes.filter((p) => !p.success).length;
    session.analysis.artifact_error_count = session.probes.filter((p) => p.artifact_error).length;
    const incomplete = session.halted || session.analysis.artifact_error_count > 0 ||
      (['units', 'row-heights'].includes(fixture.kind) && session.analysis.render_error_count > 0) ||
      (fixture.kind === 'markers' && (!session.analysis.vertical_marker_control_ok || !session.analysis.horizontal_marker_control_ok ||
        session.analysis.vertical_marker_evidence?.some((p) => p.verdict.conclusion.startsWith('inconclusive')) ||
        session.analysis.horizontal_marker_evidence?.conclusion.startsWith('inconclusive') ||
        session.analysis.synthetic_height_plateau?.conclusion === 'inconclusive' ||
        session.analysis.synthetic_width_plateau?.conclusion === 'inconclusive'));
    await session.complete(incomplete ? 'inconclusive' : 'complete');
  } catch (e) { await D.finishFailure(session, e); }
}());
