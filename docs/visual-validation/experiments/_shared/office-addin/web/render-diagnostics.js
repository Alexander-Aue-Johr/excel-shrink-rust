/* global Excel, Office, ExperimentUi */
'use strict';
// Shared only by diagnostic experiments. Never used by production capture code.
(function (root) {
  const VERSION = 'render-diagnostics-v2';
  const sleep = (ms) => new Promise((resolve) => root.setTimeout(resolve, ms));
  const finite = (v) => v !== null && v !== undefined && Number.isFinite(Number(v));
  const rounded = (v, digits = 6) => finite(v) ? Number(Number(v).toFixed(digits)) : null;
  function columnName(n) {
    if (!Number.isInteger(n) || n < 1 || n > 16384) throw new Error('Invalid 1-based column');
    let s = '';
    for (; n > 0; n = Math.floor((n - 1) / 26)) s = String.fromCharCode(65 + (n - 1) % 26) + s;
    return s;
  }
  function errorDetail(error) {
    const d = error && error.debugInfo;
    let debugInfo = null;
    try { debugInfo = d ? JSON.parse(JSON.stringify(d)) : null; }
    catch (_) { debugInfo = { serialization_error: 'debugInfo could not be serialized' }; }
    return {
      message: String(error && error.message || error), code: error && error.code || null,
      error_location: d && d.errorLocation || null, statement: d && d.statement || null,
      debug_info: debugInfo,
      trace_messages: error && Array.isArray(error.traceMessages) ? error.traceMessages.slice() : [],
      stack: error && error.stack || null
    };
  }
  function pngHeader(base64) {
    const p = { valid_header: false, base64_chars: typeof base64 === 'string' ? base64.length : 0,
      png_width: null, png_height: null, decoded_bytes_estimate: null, header_error: null,
      check: 'signature + IHDR + positive dimensions + IEND; not a full pixel decode' };
    try {
      if (typeof base64 !== 'string' || base64.length < 60 || base64.length % 4 !== 0) {
        throw new Error('Empty, short or misaligned Base64 payload');
      }
      const h = root.atob(base64.slice(0, 48));
      const b = (i) => h.charCodeAt(i);
      const u32 = (i) => b(i) * 16777216 + b(i + 1) * 65536 + b(i + 2) * 256 + b(i + 3);
      if (h.slice(0, 8) !== '\x89PNG\r\n\x1a\n' || u32(8) !== 13 || h.slice(12, 16) !== 'IHDR') {
        throw new Error('Missing PNG signature/IHDR');
      }
      p.png_width = u32(16); p.png_height = u32(20);
      if (!(p.png_width > 0 && p.png_width <= 2147483647 && p.png_height > 0 && p.png_height <= 2147483647)) {
        throw new Error('Invalid PNG dimensions');
      }
      const end = root.atob(base64.slice(-32));
      if (!end.endsWith('\x00\x00\x00\x00IEND\xaeB\x60\x82')) throw new Error('PNG has no terminating IEND');
      p.decoded_bytes_estimate = base64.length / 4 * 3 - (base64.endsWith('==') ? 2 : base64.endsWith('=') ? 1 : 0);
      p.valid_header = true;
    } catch (e) { p.header_error = String(e.message || e); }
    return p;
  }
  // Fit an offset as well as a factor: a one-pixel outside edge must not look like anisotropic DPI.
  function fit(pairs) {
    const points = pairs.filter(([x, y]) => finite(x) && finite(y)).map(([x, y]) => [Number(x), Number(y)]);
    if (points.length < 2) return null;
    const mx = points.reduce((s, p) => s + p[0], 0) / points.length;
    const my = points.reduce((s, p) => s + p[1], 0) / points.length;
    const xx = points.reduce((s, p) => s + (p[0] - mx) ** 2, 0);
    if (xx <= 0) return null;
    const slope = points.reduce((s, p) => s + (p[0] - mx) * (p[1] - my), 0) / xx;
    const intercept = my - slope * mx;
    const residuals = points.map(([x, y]) => y - (slope * x + intercept));
    return { slope, intercept, sample_count: points.length,
      max_abs_residual: Math.max(...residuals.map(Math.abs)),
      rmse: Math.sqrt(residuals.reduce((s, x) => s + x * x, 0) / residuals.length),
      model: 'output = slope * input + intercept' };
  }
  function classify(probe, axis, plateauPx, model, tolerance) {
    if (!probe || !probe.success) return 'unknown-error';
    const actual = probe[axis === 'y' ? 'png_height' : 'png_width'];
    const points = probe[axis === 'y' ? 'range_height_points' : 'range_width_points'];
    if (actual === plateauPx) return 'at-plateau';
    if (actual > plateauPx) return 'exceeds-candidate';
    if (Math.abs(actual - (model.slope * points + model.intercept)) <= tolerance) return 'below-plateau-full-scale';
    return 'other-scaling-anomaly';
  }
  class Session {
    constructor(config, workbook) {
      this.config = config; this.options = config.diagnostic_options || {};
      this.workbook = workbook; this.probes = []; this.captures = [];
      this.analysis = {}; this.halted = false; this.clientId = 'pane-' + Date.now() + '-' + Math.random().toString(16).slice(2);
      this.environment = { browser_user_agent: root.navigator && root.navigator.userAgent || null,
        device_pixel_ratio: root.devicePixelRatio || null,
        office_diagnostics: root.Office && root.Office.context && root.Office.context.diagnostics
          ? { host: root.Office.context.diagnostics.host, platform: root.Office.context.diagnostics.platform, version: root.Office.context.diagnostics.version } : null,
        note: 'devicePixelRatio is browser context information, not assumed to be the PNG DPI' };
    }
    async post(path, body) {
      const r = await root.fetch(path, { method: 'POST', headers: {
        'Content-Type': 'application/json', 'X-VV-Run-Id': this.config.diagnostic_run_id
      }, body: JSON.stringify(body) });
      if (!r.ok) throw new Error(`${path}: HTTP ${r.status}: ${await r.text()}`);
      return r.json();
    }
    async emit(stage, detail) {
      await this.post('/client-event', { stage, detail: {
        workbook: this.workbook, experiment: this.config.experiment,
        diagnostic_run_id: this.config.diagnostic_run_id, client_id: this.clientId, ...detail
      } });
    }
    snapshot(status = 'running') {
      return { schema: VERSION, workbook: this.workbook, experiment: this.config.experiment,
        diagnostic_run_id: this.config.diagnostic_run_id, status, environment: this.environment,
        options: this.options, analysis: this.analysis, probes: this.probes, captures: this.captures,
        probe_count: this.probes.length, capture_count: this.captures.length, batches: [] };
    }
    async checkpoint() { await this.post('/diagnostic-progress', this.snapshot()); }
    async complete(status = 'complete') {
      const payload = this.snapshot(status);
      root.ExperimentUi.log(`Diagnostic status: ${status}; ${this.probes.length} actual getImage attempts.`);
      await this.emit('diagnostic_completed', { status, analysis: this.options.suite === 'row-heights'
        ? this.analysis.row_height_summary || { partial: true, fatal_error: this.analysis.fatal_error } : this.analysis });
      await this.post('/result', payload);
      root.ExperimentUi.updateProgress(this.probes.length, this.probes.length);
    }
    async activate(sheetName) {
      if (this.halted) throw new Error('Diagnostic stopped after an unresolved Office request');
      await root.Excel.run(async (context) => {
        context.workbook.worksheets.getItem(sheetName).activate();
        await context.sync();
      });
      await this.emit('diagnostic_sheet_activated', { sheet: sheetName });
    }
    async probe(spec, options = {}) {
      if (this.halted) throw new Error('No more Excel calls after timeout or a failed recovery control');
      const sequence = this.probes.length + 1;
      const record = { sequence, workbook: this.workbook, ...spec, success: false,
        stage: 'pending', range_width_points: null, range_height_points: null,
        png_width: null, png_height: null, base64_chars: 0, error: null };
      const label = `#${sequence} ${spec.role} sheet=${JSON.stringify(spec.sheet)} range=${spec.address}`;
      root.ExperimentUi.setPhase(label);
      await this.emit('diagnostic_probe_started', record);
      let captured = null;
      let timer;
      const timeoutMs = Math.max(10000, Number(this.options.probe_timeout_ms || 240000));
      const started = root.performance.now();
      const operation = root.Excel.run(async (context) => {
        const range = context.workbook.worksheets.getItem(spec.sheet).getRange(spec.address);
        const loadGeometry = async () => {
          const geometryStarted = root.performance.now();
          record.stage = 'load-geometry';
          range.load('address,width,height,left,top,rowCount,columnCount');
          await context.sync();
          Object.assign(record, {
            resolved_address: range.address, range_width_points: Number(range.width),
            range_height_points: Number(range.height), range_left_points: Number(range.left),
            range_top_points: Number(range.top), row_count: Number(range.rowCount), column_count: Number(range.columnCount),
            expected_width_px_96dpi: rounded(Number(range.width) * 96 / 72),
            expected_height_px_96dpi: rounded(Number(range.height) * 96 / 72),
            geometry_ms: rounded(root.performance.now() - geometryStarted)
          });
        };
        const imageFirst = options.geometryOrder === 'image-first';
        record.geometry_timing = imageFirst ? 'after-getImage' : 'before-getImage';
        if (!imageFirst) await loadGeometry();
        record.stage = 'get-image';
        context.trace(`${VERSION} BEFORE getImage sheet=${JSON.stringify(spec.sheet)} address=${spec.address}`);
        const renderStarted = root.performance.now();
        const image = range.getImage();
        context.trace(`${VERSION} AFTER getImage sheet=${JSON.stringify(spec.sheet)} address=${spec.address}`);
        try { await context.sync(); }
        catch (error) {
          // Inspect record, spec, error.debugInfo and traceMessages in DevTools.
          if (this.options.break_on_error || root.VV_BREAK_ON_ERROR) { debugger; }
          throw error;
        }
        record.render_ms = rounded(root.performance.now() - renderStarted);
        if (imageFirst) await loadGeometry();
        record.stage = 'payload';
        const h = pngHeader(image.value);
        Object.assign(record, h);
        record.success = h.valid_header;
        record.stage = h.valid_header ? 'success' : 'invalid-png';
        if (!record.success) {
          record.error = { message: h.header_error, code: 'InvalidPngPayload' };
          if (this.options.break_on_error || root.VV_BREAK_ON_ERROR) { debugger; }
        }
        return { base64: image.value };
      });
      try {
        captured = await Promise.race([operation, new Promise((_, reject) => {
          timer = root.setTimeout(() => {
            const error = new Error('getImage/load timeout; request is NOT cancelled. No further Excel calls will be queued.');
            error.code = 'DiagnosticTimeout'; reject(error);
          }, timeoutMs);
        })]);
      } catch (error) {
        record.success = false;
        record.error = errorDetail(error);
        if (error.code === 'DiagnosticTimeout') { this.halted = true; record.stage = 'timeout'; }
        console.error('Diagnostic getImage attempt failed', { record, error });
        if (this.options.break_on_error || root.VV_BREAK_ON_ERROR) { debugger; }
      } finally {
        root.clearTimeout(timer);
        record.duration_ms = rounded(root.performance.now() - started);
      }
      // Save a detached JSON record: a timed-out Office promise may resolve much later.
      const saved = JSON.parse(JSON.stringify(record));
      this.probes.push(saved);
      if (saved.success) {
        saved.width_px_per_point_raw = rounded(saved.png_width / saved.range_width_points);
        saved.height_px_per_point_raw = rounded(saved.png_height / saved.range_height_points);
        saved.width_delta_from_96dpi_px = rounded(saved.png_width - saved.expected_width_px_96dpi);
        saved.height_delta_from_96dpi_px = rounded(saved.png_height - saved.expected_height_px_96dpi);
      }
      try {
        if (saved.success && options.persist && captured) {
          const id = 'd' + String(sequence).padStart(5, '0');
          const result = await this.post('/image-batch', { workbook: this.workbook,
            batch_index: sequence, images: [{ id, sheet: spec.sheet, address: spec.address, base64: captured.base64 }] });
          this.captures.push(...result.captures || []);
          saved.artifact_id = id;
          saved.relative_png = result.captures && result.captures[0] && result.captures[0].relative_png;
          if (options.markers) {
            saved.marker_analysis = await this.post('/diagnostic-markers', {
              workbook: this.workbook, id, axis: options.markerAxis || 'y', markers: options.markers
            });
          }
        }
      } catch (e) {
        // A decoder/HTTP failure is not a failed Range.getImage result.
        saved.artifact_error = errorDetail(e);
        console.error('Diagnostic artifact/marker analysis failed (getImage outcome is unchanged)', saved);
      } finally { if (captured) captured.base64 = null; captured = null; }
      root.ExperimentUi.log(`${label} => ${saved.success ? `${saved.png_width}x${saved.png_height} px` : saved.stage + ': ' + saved.error.message}`);
      root.ExperimentUi.updateProgress(this.probes.length, Math.max(1, this.probes.length + 1));
      await this.emit('diagnostic_probe', saved);
      await this.checkpoint();
      await sleep(100);
      return saved;
    }
    async reliable(spec, controlAddress) {
      let p = await this.probe(spec);
      if (p.success) return p;
      if (this.halted || p.stage === 'load-geometry') return null;
      const control = await this.probe({ sheet: spec.sheet, address: controlAddress,
        role: `recovery-control-after-${p.sequence}` });
      if (!control.success) { this.halted = true; return null; }
      p = await this.probe({ ...spec, role: spec.role + '-retry' });
      return p.success ? p : null;
    }
  }
  async function open(experiment) {
    const configResponse = await root.fetch('/config', { cache: 'no-store' });
    if (!configResponse.ok) throw new Error('Cannot read diagnostic config');
    const config = await configResponse.json();
    if (config.experiment !== experiment || !config.diagnostic_run_id) throw new Error('Diagnostic mode/config mismatch');
    const workbook = await root.Excel.run(async (c) => { c.workbook.load('name'); await c.sync(); return c.workbook.name; });
    const session = new Session(config, workbook);
    const claim = await session.post('/diagnostic-claim', { workbook, client_id: session.clientId });
    if (!claim.acquired) { root.ExperimentUi.setPhase('Another task pane owns this diagnostic run; this pane will not run probes.'); return null; }
    root.ExperimentUi.setWorkbook(workbook);
    await session.emit('measurement_started', { environment: session.environment });
    return session;
  }
  async function finishFailure(session, error) {
    console.error('Diagnostic incomplete', error);
    if (!session) { root.ExperimentUi.fail(String(error.message || error)); return; }
    session.analysis.fatal_error = errorDetail(error);
    root.ExperimentUi.setPhase('Diagnostic inconclusive: ' + String(error.message || error));
    await session.complete('inconclusive');
  }
  // A rendering exception is UNKNOWN for a plateau predicate, never evidence for a cap.
  async function plateauSearch(session, options) {
    const { sheet, axis, maxIndex, baseline, address, controlAddress } = options;
    const dim = axis === 'y' ? 'png_height' : 'png_width';
    const geom = axis === 'y' ? 'range_height_points' : 'range_width_points';
    const observations = [];
    const result = { axis, sheet, max_test_index: maxIndex, conclusion: 'inconclusive',
      first_plateau_index: null, last_below_plateau_index: null, plateau_px: null,
      assumption: 'monotone saturation; checked at sampled points and repeated near the transition, not globally proved' };
    const probe = async (index, role) => {
      const p = await session.reliable({ sheet, address: address(index), role, search_axis: axis, search_index: index }, controlAddress);
      if (!p) throw new Error(`No reliable image for ${sheet}!${address(index)}. Error is not classified as plateau.`);
      observations.push(p); return p;
    };
    try {
      for (const index of baseline) await probe(index, 'scale-calibration');
      const model = fit(observations.map((p) => [p[geom], p[dim]]));
      result.calibration = model;
      if (!model || model.slope <= 0 || model.max_abs_residual > 3) throw new Error('Small-image scale calibration is not linear/reliable');
      const tolerance = Math.max(2, model.max_abs_residual * 3 + 1);
      result.calibration_tolerance_px = tolerance;
      const full = await probe(maxIndex, 'plateau-upper-bound');
      const expected = model.slope * full[geom] + model.intercept;
      result.full_range = full;
      if (full[dim] >= expected - tolerance) {
        result.conclusion = 'no-saturation-observed-up-to-test-limit'; return result;
      }
      const cap = full[dim];
      result.plateau_px = cap;
      const again = await probe(maxIndex, 'plateau-upper-bound-repeat');
      if (again[dim] !== cap) throw new Error('Full range returns different dimensions on repetition');
      let low = baseline[0]; let high = maxIndex;
      const first = observations[0];
      if (classify(first, axis, cap, model, tolerance) !== 'below-plateau-full-scale') throw new Error('No below-plateau lower bound');
      while (high - low > 1) {
        const mid = Math.floor((low + high) / 2);
        const p = await probe(mid, 'plateau-binary-search');
        const kind = classify(p, axis, cap, model, tolerance);
        if (kind === 'at-plateau') high = mid;
        else if (kind === 'below-plateau-full-scale') low = mid;
        else throw new Error('Non-monotone/other scale anomaly at ' + p.address + ': ' + kind);
      }
      result.first_plateau_index = high; result.last_below_plateau_index = low;
      result.first_plateau_address = address(high); result.last_below_plateau_address = address(low);
      const radius = options.neighborhoodRadius === undefined ? 5 : options.neighborhoodRadius;
      const local = [];
      for (let i = Math.max(1, high - radius); i <= Math.min(maxIndex, high + radius); i++) {
        const p = await probe(i, 'plateau-neighborhood');
        local.push(p);
        const kind = classify(p, axis, cap, model, tolerance);
        if (kind !== (i < high ? 'below-plateau-full-scale' : 'at-plateau')) throw new Error('Boundary not reproduced during local sweep');
      }
      result.neighborhood = local;
      for (const i of [high, low, high, low]) {
        const p = await probe(i, 'plateau-boundary-repeat');
        if (classify(p, axis, cap, model, tolerance) !== (i === low ? 'below-plateau-full-scale' : 'at-plateau')) {
          throw new Error('Boundary not stable during alternating repetitions');
        }
      }
      const distinctPlateauPoints = new Set(observations.filter((p) => p[dim] === cap).map((p) => p[geom]));
      if (distinctPlateauPoints.size < 2) throw new Error('Only one requested physical extent reached the candidate; plateau is not confirmed');
      const byIndex = observations.slice().sort((a, b) => a.search_index - b.search_index);
      if (byIndex.some((p, i) => i && p[dim] < byIndex[i - 1][dim])) throw new Error('Observed PNG dimension is not monotone');
      result.conclusion = 'plateau-boundary-locally-confirmed';
      result.confirmed_at_distinct_requested_extents = distinctPlateauPoints.size;
      result.last_below_plateau_png_px = local.find((p) => p.search_index === low)?.[dim] ?? null;
      result.first_plateau_requested_points = local.find((p) => p.search_index === high)?.[geom] ?? null;
    } catch (e) { result.conclusion = 'inconclusive'; result.reason = errorDetail(e); }
    finally { result.probe_sequences = observations.map((p) => p.sequence); }
    return result;
  }
  function markerVerdict(probe, markers, axis, model) {
    const data = probe.marker_analysis;
    if (!probe.success || !data || !Array.isArray(data.markers) || !model) return { conclusion: 'inconclusive-no-marker-data' };
    const dimension = axis === 'y' ? probe.png_height : probe.png_width;
    const totalPoints = axis === 'y' ? probe.range_height_points : probe.range_width_points;
    const comparisons = markers.map((m) => {
      const o = data.markers.find((v) => v.name === m.name);
      const center = (m.start_points + m.end_points) / 2;
      return { name: m.name, observed: !!o && o.count > 0,
        observed_center_px: o && o.count ? (o.first + o.last) / 2 : null,
        segments: o ? o.segments : [],
        native_center_px: center * model.slope + model.intercept / 2,
        scaled_center_px: center / totalPoints * (dimension - 1) + 0.5,
        wholly_after_native_cap: m.start_points * model.slope > dimension + 3 };
    });
    const found = comparisons.filter((m) => m.observed && m.segments.length === 1);
    const nativeRmse = found.length ? Math.sqrt(found.reduce((s, m) => s + (m.observed_center_px - m.native_center_px) ** 2, 0) / found.length) : null;
    const scaledRmse = found.length ? Math.sqrt(found.reduce((s, m) => s + (m.observed_center_px - m.scaled_center_px) ** 2, 0) / found.length) : null;
    const late = comparisons.filter((m) => m.wholly_after_native_cap);
    const reduced = dimension < model.slope * totalPoints + model.intercept - 4;
    let conclusion = 'inconclusive';
    if (!reduced) conclusion = 'no-dimension-reduction-in-this-probe';
    else if (found.length >= 3 && nativeRmse < 4 && late.length >= 2 && late.every((m) => !m.observed)) conclusion = 'consistent-with-clipping';
    else if (found.length >= 3 && comparisons.every((m) => m.observed) && scaledRmse < 4 && nativeRmse > scaledRmse * 3) conclusion = 'consistent-with-scaling-on-tested-axis';
    return { conclusion, native_rmse_px: nativeRmse, scaled_rmse_px: scaledRmse, comparisons,
      note: 'Evidence from solid-fill markers at known measured positions; unexpected patterns remain inconclusive.' };
  }
  const api = { VERSION, rounded, columnName, pngHeader, errorDetail, fit, classify, Session, open,
    finishFailure, plateauSearch, markerVerdict };
  root.VVRenderDiagnostics = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
}(typeof window !== 'undefined' ? window : globalThis));
