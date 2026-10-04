/* global Excel, VVRenderDiagnostics */
'use strict';
// Diagnostic only. No changes to production screenshot planning or rendering.
(function (root) {
  const D = root.VVRenderDiagnostics || (typeof require === 'function' ? require('./render-diagnostics.js') : null);
  const finite = (v) => v !== null && v !== undefined && Number.isFinite(Number(v));
  const num = (v) => finite(v) ? Number(v) : null;
  const sum = (a) => a.every(finite) ? a.reduce((s, x) => s + Number(x), 0) : null;
  const difference = (a, b) => finite(a) && finite(b) ? D.rounded(a - b) : null;
  const copy = (x) => JSON.parse(JSON.stringify(x));
  const sleep = (ms) => new Promise((resolve) => root.setTimeout(resolve, ms));

  async function office(session, label, operation) {
    if (session.halted) throw new Error('No further Office calls after an unresolved request');
    let timer;
    const pending = root.Excel.run(operation);
    try {
      return await Promise.race([pending, new Promise((_, reject) => {
        timer = root.setTimeout(() => {
          session.halted = true;
          const e = new Error(label + ': timeout; Office request is not cancelled. No further Office calls will be queued.');
          e.code = 'DiagnosticTimeout'; reject(e);
        }, Math.max(10000, Number(session.options.probe_timeout_ms || 240000)));
      })]);
    } catch (error) {
      console.error('Row-height diagnostic Office failure', { label, error });
      if (session.options.break_on_error || root.VV_BREAK_ON_ERROR) { debugger; }
      throw error;
    } finally { root.clearTimeout(timer); }
  }

  function rowSource(profile, row) {
    const c = profile.cases && profile.cases.find((x) => x.row === row);
    return {
      case_id: c ? c.id : profile.pair || profile.sheet,
      xml_row_present: c ? c.xml_row_present : true,
      xml_ht: c ? c.xml_ht : profile.heights ? profile.heights[row - 1] : profile.row_ht ?? null,
      xml_custom_height: c ? c.xml_custom_height : profile.row_custom_height ?? null,
      xml_row_style: c ? c.row_style ?? null : null
    };
  }

  function aggregate(r) {
    return { height_points: num(r.height), width_points: num(r.width), top_points: num(r.top),
      row_count: num(r.rowCount), address: r.address };
  }

  async function snapshot(session, profile, phase, study) {
    const count = Number(profile.row_count);
    if (!Number.isInteger(count) || count < 1 || count > 1200) throw new Error('Invalid row snapshot count');
    const index = session.analysis.row_height.snapshots.length + 1;
    const label = `${study} sheet=${JSON.stringify(profile.sheet)} phase=${phase}`;
    await session.emit('row_geometry_started', { sheet: profile.sheet, phase, study, snapshot_id: index, row_count: count });
    const started = root.performance.now();
    const data = await office(session, label, async (context) => {
      const ws = context.workbook.worksheets.getItem(profile.sheet);
      const block = ws.getRange(profile.address);
      block.load('address,height,width,top,rowCount');
      context.trace('ROW-HEIGHT before aggregate read ' + label);
      await context.sync();
      const before = aggregate(block);
      // Query EVERY row, not a representative row. Font queries only for the small case matrix.
      const requests = [];
      for (let row = 1; row <= count; row++) {
        const r = ws.getRange(`A${row}:${profile.last_column || 'A'}${row}`);
        r.load('top,height,rowHidden');
        r.format.load('rowHeight');
        let a = null, c = null;
        if (profile.cases || study === 'runtime-font-change') {
          a = ws.getRange(`A${row}`).format.font;
          c = ws.getRange(`C${row}`).format.font;
          a.load('name,size'); c.load('name,size');
        }
        requests.push({ row, r, a, c });
      }
      const next = ws.getRange(`A${count + 1}`); next.load('top');
      context.trace('ROW-HEIGHT before per-row read ' + label);
      await context.sync();
      const rows = requests.map(({ row, r, a, c }) => ({
        row, ...rowSource(profile, row), top_points: num(r.top), height_points: num(r.height),
        format_row_height_points: num(r.format.rowHeight), hidden: r.rowHidden,
        ...(a ? { font_A: { name: a.name, size_pt: num(a.size) }, font_C: { name: c.name, size_pt: num(c.size) } } : {})
      }));
      const nextTop = num(next.top);
      // Reload after individual reads so a query-induced geometry change is not silently lost.
      block.load('address,height,width,top,rowCount');
      await context.sync();
      const after = aggregate(block);
      const heights = sum(rows.map((x) => x.height_points));
      const formats = sum(rows.map((x) => x.format_row_height_points));
      return { snapshot_id: index, study, sheet: profile.sheet, phase,
        default_row_height_xml_pt: profile.default_row_height_pt ?? null,
        sheet_custom_height_xml: profile.sheet_custom_height ?? null,
        source_attributes_are_original_xml_not_reloaded_after_edits: true,
        aggregate_before_row_reads: before, aggregate_after_row_reads: after,
        sum_individual_height_points: heights, sum_format_row_height_points: formats,
        height_from_next_row_top_points: difference(nextTop, rows[0].top_points),
        aggregate_change_during_reads_points: difference(after.height_points, before.height_points),
        sum_minus_aggregate_after_points: difference(heights, after.height_points), rows };
    });
    data.duration_ms = D.rounded(root.performance.now() - started);
    // Detach to avoid later proxy or timeout mutations entering completed measurements.
    const saved = copy(data);
    session.analysis.row_height.snapshots.push(saved);
    await session.emit('row_geometry_snapshot', { sheet: profile.sheet, phase, study,
      snapshot_id: index, row_count: count,
      aggregate_before_points: saved.aggregate_before_row_reads.height_points,
      aggregate_after_points: saved.aggregate_after_row_reads.height_points,
      sum_individual_height_points: saved.sum_individual_height_points,
      sum_minus_aggregate_after_points: saved.sum_minus_aggregate_after_points,
      aggregate_change_during_reads_points: saved.aggregate_change_during_reads_points });
    await session.checkpoint();
    return saved;
  }

  function requireImage(session, p) {
    if (!p.success || session.halted) {
      // No retry in these causal sequences: retrying would change the state being measured.
      throw new Error(`Sequence cannot be interpreted after failed image: ${p.sheet}!${p.address}; ${p.stage}`);
    }
    return p;
  }

  function summarizePartitions(whole, parts) {
    if (!whole || !whole.success || !parts.length || parts.some((p) => !p.success)) return { status: 'inconclusive' };
    const total = sum(parts.map((p) => p.png_height));
    return { status: 'measured', whole_sequence: whole.sequence, part_sequences: parts.map((p) => p.sequence),
      whole_png_height: whole.png_height, sum_part_png_heights: total,
      inferred_extra_pixel_per_capture: parts.length > 1 ? (total - whole.png_height) / (parts.length - 1) : null,
      residuals_by_assumed_extra_pixel: [0, 1, 2].map((edge) => ({ extra_pixel: edge,
        predicted_whole_height: total - (parts.length - 1) * edge,
        residual_px: total - (parts.length - 1) * edge - whole.png_height })),
      note: 'Additivity test, not a stitched bitmap. A +1 outside pixel is a hypothesis, not a universal guarantee.' };
  }

  function summarizeMatrix(profile, rowImages, before, after) {
    const rows = rowImages.map((p, i) => ({
      row: i + 1, case_id: profile.cases[i].id, sequence: p.sequence,
      png_height: p.success ? p.png_height : null, api_height_at_image: p.range_height_points,
      before_height_points: before.rows[i].height_points, after_height_points: after.rows[i].height_points,
      height_change_points: difference(after.rows[i].height_points, before.rows[i].height_points),
      font_A: after.rows[i].font_A, font_C: after.rows[i].font_C
    }));
    const lookup = (id) => rows.find((r) => r.case_id === id)?.png_height ?? null;
    const tested = ['text-font8', 'text-font11', 'text-font24', 'text-font48'].map(lookup);
    return { sheet: profile.sheet, default_row_height_xml_pt: profile.default_row_height_pt,
      sheet_custom_height_xml: profile.sheet_custom_height,
      row_ht_xml: profile.row_ht, row_custom_height_xml: profile.row_custom_height,
      font8_vs_empty_delta_px: difference(lookup('text-font8'), lookup('row-only')),
      font48_vs_font8_delta_px: difference(lookup('text-font48'), lookup('text-font8')),
      blank_font48_vs_unformatted_blank_delta_px: difference(lookup('blank-font48'), lookup('blank-cell-no-style')),
      span_over_text_font_sizes_px: tested.every(finite) ? Math.max(...tested) - Math.min(...tested) : null,
      note: 'Differences are observations, not forced pass/fail assertions about minimum height.', rows };
  }

  async function matrix(session, profile) {
    const entry = { sheet: profile.sheet, profile, snapshot_ids: [], probe_sequences: [] };
    session.analysis.row_height.matrix.push(entry);
    const snap = async (phase) => {
      const s = await snapshot(session, profile, phase, 'font-height-matrix'); entry.snapshot_ids.push(s.snapshot_id); return s;
    };
    // No explicit activation or AutoFit: observe the OOXML loaded by Excel as-is.
    const before = await snap('before-first-render');
    const first = requireImage(session, await session.probe({ sheet: profile.sheet, address: profile.address,
      role: 'row-height-matrix-whole-first' }, { persist: true }));
    entry.probe_sequences.push(first.sequence);
    await snap('after-first-render');
    const repeat = requireImage(session, await session.probe({ sheet: profile.sheet, address: profile.address,
      role: 'row-height-matrix-whole-repeat' }, { persist: true }));
    entry.probe_sequences.push(repeat.sequence);
    await snap('after-repeat');
    const images = [];
    for (const c of profile.cases) {
      const persist = ['text-font48', 'blank-font48', 'text-font8', 'wrap-font24-two-lines'].includes(c.id);
      const p = requireImage(session, await session.probe({ sheet: profile.sheet,
        address: `A${c.row}:D${c.row}`, role: 'row-height-case', row_case_id: c.id,
        xml_row_height_points: c.xml_ht, xml_custom_height: c.xml_custom_height,
        xml_default_row_height_points: profile.default_row_height_pt,
        xml_sheet_custom_height: profile.sheet_custom_height }, { persist }));
      images.push(p); entry.probe_sequences.push(p.sequence);
    }
    const after = await snap('after-individual-row-images');
    entry.summary = summarizeMatrix(profile, images, before, after);
    entry.whole_repeat_height_delta_px = difference(repeat.png_height, first.png_height);
    entry.individual_to_whole_additivity = summarizePartitions(repeat, images);
    await session.emit('row_height_matrix_summary', entry.summary);
    await session.checkpoint();
  }

  async function geometry(session, profile) {
    const entry = { sheet: profile.sheet, pair: profile.pair, measurement_order: profile.measurement_order,
      snapshot_ids: [], probe_sequences: [], initial_sheet_activation_requested: false };
    session.analysis.row_height.geometry.push(entry);
    const snap = async (phase) => {
      const s = await snapshot(session, profile, phase, 'geometry-read-order'); entry.snapshot_ids.push(s.snapshot_id); return s;
    };
    if (profile.measurement_order === 'geometry-first') await snap('before-first-render');
    const first = requireImage(session, await session.probe({ sheet: profile.sheet, address: profile.address,
      role: 'row-geometry-initial', measurement_order: profile.measurement_order },
      { persist: true, geometryOrder: profile.measurement_order }));
    entry.probe_sequences.push(first.sequence);
    const afterFirst = await snap('after-first-render');
    const repeat = requireImage(session, await session.probe({ sheet: profile.sheet, address: profile.address,
      role: 'row-geometry-repeat-after-row-reads' }, { persist: true }));
    entry.probe_sequences.push(repeat.sequence);
    await snap('after-repeat');
    // This is a separately labelled intervention. It is not silently applied to a cold capture.
    await office(session, 'activate ' + profile.sheet, async (c) => {
      c.workbook.worksheets.getItem(profile.sheet).activate(); await c.sync();
    });
    await sleep(200);
    await snap('after-explicit-activation');
    const active = requireImage(session, await session.probe({ sheet: profile.sheet, address: profile.address,
      role: 'row-geometry-after-explicit-activation' }, { persist: true }));
    entry.probe_sequences.push(active.sequence);
    // Pixel sum of disjoint parts can test per-row raster rounding without 1000 individual screenshots.
    const parts = [];
    for (const [start, end] of [[1,32],[33,64],[65,100],[101,200],[201,500],[501,1000]]) {
      const p = requireImage(session, await session.probe({ sheet: profile.sheet, address: `A${start}:A${end}`,
        role: 'row-height-disjoint-part', first_row: start, last_row: end }));
      parts.push(p); entry.probe_sequences.push(p.sequence);
    }
    entry.part_additivity = summarizePartitions(active, parts);
    entry.individual_samples = [];
    for (const row of [1,2,16,32,33,64,128,512,1000]) {
      const p = requireImage(session, await session.probe({ sheet: profile.sheet, address: `A${row}:A${row}`,
        role: 'row-geometry-individual-sample', sampled_row: row }));
      entry.individual_samples.push({ row, sequence: p.sequence, png_height: p.png_height,
        api_height_at_image: p.range_height_points,
        height_after_first_render: afterFirst.rows[row - 1].height_points });
      entry.probe_sequences.push(p.sequence);
    }
    const after = await snap('after-parts-and-samples');
    const final = requireImage(session, await session.probe({ sheet: profile.sheet, address: profile.address,
      role: 'row-geometry-final-repeat' }));
    entry.probe_sequences.push(final.sequence);
    entry.png_height_timeline = [first,repeat,active,final].map((p) => ({ sequence:p.sequence, role:p.role, png_height:p.png_height }));
    entry.total_height_change_after_first_points = difference(after.aggregate_after_row_reads.height_points, afterFirst.aggregate_after_row_reads.height_points);
    entry.individual_row_changes = after.rows.filter((r,i) => difference(r.height_points, afterFirst.rows[i].height_points) !== 0)
      .map((r) => ({ row:r.row, before:afterFirst.rows[r.row - 1].height_points, after:r.height_points }));
    entry.final_part_additivity = summarizePartitions(final, parts);
    await session.emit('row_height_geometry_summary', { sheet:profile.sheet, order:profile.measurement_order,
      png_height_timeline:entry.png_height_timeline, changed_individual_rows:entry.individual_row_changes.length,
      aggregate_change_points:entry.total_height_change_after_first_points,
      final_part_additivity:entry.final_part_additivity });
    await session.checkpoint();
  }

  async function runtime(session, profile) {
    const entry = { sheet:profile.sheet, profile, steps:[], source_xml_attributes_are_pre_edit_only:true };
    session.analysis.row_height.runtime.push(entry);
    for (const action of ['as-opened','set-font48','set-font8','explicit-autofit']) {
      if (action !== 'as-opened') {
        await session.emit('row_height_intervention', { sheet:profile.sheet, action });
        await office(session, `${profile.sheet} ${action}`, async (c) => {
          const ws = c.workbook.worksheets.getItem(profile.sheet);
          if (action === 'explicit-autofit') ws.getRange('A1:D1').format.autofitRows();
          else ws.getRange('A1').format.font.size = action === 'set-font48' ? 48 : 8;
          await c.sync();
        });
      }
      const before = await snapshot(session,profile,`${action}-before-render`,'runtime-font-change');
      const p = requireImage(session,await session.probe({ sheet:profile.sheet,address:profile.address,
        role:'row-height-runtime',intervention:action }, { persist:true }));
      const after = await snapshot(session,profile,`${action}-after-render`,'runtime-font-change');
      entry.steps.push({ action, snapshot_before:before.snapshot_id, snapshot_after:after.snapshot_id,
        sequence:p.sequence,png_height:p.png_height,api_height_points:p.range_height_points });
      await session.checkpoint();
    }
  }

  async function rounding(session,profile) {
    const entry = { profile, observations:[] }; session.analysis.row_height.rounding = entry;
    const before = await snapshot(session,profile,'before-first-render','rounding-thresholds');
    const whole = requireImage(session,await session.probe({ sheet:profile.sheet,address:profile.address,
      role:'row-rounding-whole' }, { persist:true }));
    const images=[];
    for (let i=0;i<profile.heights.length;i++) {
      const h=profile.heights[i];
      const p=requireImage(session,await session.probe({ sheet:profile.sheet,address:`A${i+1}:A${i+1}`,
        role:'row-rounding-threshold',xml_row_height_points:h,xml_custom_height:1 }));
      images.push(p);
      entry.observations.push({ row:i+1,xml_ht:h,sequence:p.sequence,api_height_at_image:p.range_height_points,
        png_height:p.png_height,
        predictions_96dpi_with_extra_pixel1:{ floor:Math.floor(h*4/3+1e-9)+1,round:Math.round(h*4/3)+1,ceil:Math.ceil(h*4/3-1e-9)+1 } });
    }
    const after=await snapshot(session,profile,'after-single-rows','rounding-thresholds');
    entry.snapshot_ids=[before.snapshot_id,after.snapshot_id];
    entry.additivity=summarizePartitions(whole,images);
    entry.mismatch_counts={};
    for (const model of ['floor','round','ceil']) entry.mismatch_counts[model]=entry.observations.filter((p)=>p.png_height!==p.predictions_96dpi_with_extra_pixel1[model]).length;
    await session.checkpoint();
  }

  async function run(session, fixture) {
    session.analysis.row_height = { version:'row-height-study-v1', snapshots:[],matrix:[],geometry:[],runtime:[],
      notes:[
        'defaultRowHeight as a font-dependent minimum is a user-tested hypothesis to distinguish, not hardcoded expected output.',
        'sheetFormatPr@customHeight and row@customHeight are separate factors.',
        'No explicit AutoFit except labelled runtime intervention at the very end of each runtime control.',
        'Cold means no earlier diagnostic access to that sheet; Excel may already initialize layout on workbook open.',
        'Images and XML are preserved; PNG dimensions alone do not prove glyph clipping.',
        'Font names are API-reported requested formatting, not proof against platform font substitution.'
      ] };
    for (const p of fixture.matrices || []) await matrix(session,p);
    for (const p of fixture.runtime_controls || []) await runtime(session,p);
    for (const p of fixture.geometry || []) await geometry(session,p);
    if (fixture.rounding) await rounding(session,fixture.rounding);
    const data=session.analysis.row_height;
    data.paired_order_comparisons=[];
    for (const pair of [...new Set(data.geometry.map((p)=>p.pair))]) {
      const g=data.geometry.find((p)=>p.pair===pair&&p.measurement_order==='geometry-first');
      const i=data.geometry.find((p)=>p.pair===pair&&p.measurement_order==='image-first');
      data.paired_order_comparisons.push({pair,
        geometry_first_initial_png_height:g?.png_height_timeline?.[0]?.png_height??null,
        image_first_initial_png_height:i?.png_height_timeline?.[0]?.png_height??null,
        initial_height_delta_px:difference(g?.png_height_timeline?.[0]?.png_height,i?.png_height_timeline?.[0]?.png_height),
        note:'One paired sequence per workbook opening; any difference warrants an independent repeat.'});
    }
    session.analysis.row_height_summary={
      matrix_sheets:data.matrix.length,geometry_sequences:data.geometry.length,
      snapshots:data.snapshots.length,runtime_controls:data.runtime.length,
      matrix:data.matrix.map((x)=>({sheet:x.sheet,font48_vs_font8_delta_px:x.summary.font48_vs_font8_delta_px,
        span_over_text_font_sizes_px:x.summary.span_over_text_font_sizes_px,
        row_custom_height_xml:x.summary.row_custom_height_xml,sheet_custom_height_xml:x.summary.sheet_custom_height_xml})),
      paired_order_comparisons:data.paired_order_comparisons,
      rounding_model_mismatch_counts:data.rounding?.mismatch_counts??null
    };
    await session.emit('row_height_study_completed',session.analysis.row_height_summary);
  }
  const api={ run,snapshot,rowSource,summarizePartitions,summarizeMatrix,office };
  root.VVRowHeightDiagnostics=api;
  if (typeof module!=='undefined'&&module.exports) module.exports=api;
}(typeof window!=='undefined'?window:globalThis));
