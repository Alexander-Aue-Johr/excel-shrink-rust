/* global Excel, ExperimentUi, ExperimentEvents */
'use strict';
// Capture safety is deliberately separate from content/overflow-bound discovery.
// Per-sheet small PNG controls calibrate both axes before tile planning.
// Browser DPR is context only: neither DPR, 96 DPI, nor a previous run supplies the scale.
(function (root) {
  const DEFAULTS = Object.freeze({ max_edge_px: 30000, max_pixels: 16000000,
    max_tiles: 4096, max_attempts: 8192, max_depth: 24, timeout_ms: 90000,
    scale: null, scale_x: null, scale_y: null, border_px: 1, tolerance_px: 2 });
  const ROWS = 1048576, COLS = 16384;
  function fault(code, message, detail, fatal = false) {
    const e = new Error(message); e.code = code; e.detail = detail; e.fatal = fatal; return e;
  }
  function errorInfo(e) {
    return { code: e && e.code || 'Error', message: e && e.message || String(e),
      stack: e && e.stack || null, debug_info: e && e.debugInfo || null,
      trace_messages: e && e.traceMessages || null, detail: e && e.detail || null,
      fatal: Boolean(e && e.fatal) };
  }
  function col(n) { let s = ''; for (n++; n > 0; n = Math.floor((n - 1) / 26)) s = String.fromCharCode(65 + (n - 1) % 26) + s; return s; }
  function address(r) { return `${col(r.c0)}${r.r0 + 1}:${col(r.c1 - 1)}${r.r1}`; }
  function parse(a) {
    const m = /^(\$?[A-Z]+)\$?(\d+)(?::(\$?[A-Z]+)\$?(\d+))?$/i.exec(String(a).split('!').pop());
    if (!m) throw fault('INVALID_ADDRESS', 'Expected a rectangular A1 range: ' + a);
    const n = s => Array.from(s.replace(/\$/g, '').toUpperCase()).reduce((x, c) => x * 26 + c.charCodeAt(0) - 64, 0);
    const r = { r0: Number(m[2]) - 1, r1: Number(m[4] || m[2]), c0: n(m[1]) - 1, c1: n(m[3] || m[1]) };
    if (r.r0 < 0 || r.c0 < 0 || r.r1 > ROWS || r.c1 > COLS || r.r1 <= r.r0 || r.c1 <= r.c0) throw fault('INVALID_BOUNDS', 'Invalid capture bounds', r);
    return r;
  }
  function png(base64) {
    if (typeof base64 !== 'string' || base64.length < 60) throw fault('INVALID_PNG', 'getImage returned no usable PNG', { base64_chars: base64 && base64.length });
    let h;
    try { h = atob(base64.slice(0, 44)); } catch (e) { throw fault('INVALID_PNG', 'Invalid PNG Base64 header', errorInfo(e)); }
    const b = i => h.charCodeAt(i) & 255;
    if (![137,80,78,71,13,10,26,10].every((x,i) => x === b(i)) || h.slice(12,16) !== 'IHDR') throw fault('INVALID_PNG', 'PNG signature/IHDR missing');
    const u = o => ((b(o) * 16777216) + (b(o+1) << 16) + (b(o+2) << 8) + b(o+3));
    let tail; try { tail=atob(base64.slice(-32)); } catch (_) { throw fault('INVALID_PNG','PNG tail is not Base64'); }
    if (tail.slice(-8,-4) !== 'IEND') throw fault('INVALID_PNG','PNG IEND missing (truncated image)');
    const out = { width: u(16), height: u(20), base64_chars: base64.length };
    if (!(out.width > 0 && out.height > 0)) throw fault('INVALID_PNG', 'Zero PNG dimension', out);
    return out;
  }
  function axisScale(limits, axis) {
    const value = Number(limits['scale_' + axis] == null ? limits.scale : limits['scale_' + axis]);
    if (!Number.isFinite(value) || value <= 0) throw fault('UNCALIBRATED_CAPTURE',
      'No measured raster scale for this sheet; no native capture was queued.', {axis}, true);
    return value;
  }
  // Fixed-width sheets do not identify an X scale. Keep the measured horizontal
  // interval and its observed PNG width instead; never extrapolate from one sample.
  function assertFixedSpan(g, limits) {
    const f=limits.fixed_x;
    if(!f || !Number.isFinite(g.left) || !Number.isFinite(g.right) ||
       Math.abs(g.left-f.left)>1e-8 || Math.abs(g.right-f.right)>1e-8)
      throw fault('FIXED_WIDTH_SPAN_CHANGED','This calibration covers one horizontal interval only.',{geometry:g,fixed_x:f},true);
    return f;
  }
  function xOffset(from, to, limits) {
    if(limits.fixed_x) {
      if(!Number.isFinite(from)||!Number.isFinite(to)||Math.abs(to-from)>1e-8)
        throw fault('FIXED_WIDTH_HORIZONTAL_OPERATION','A fixed-width calibration cannot place a horizontal subrange.',{from,to,fixed_x:limits.fixed_x},true);
      return 0;
    }
    return Math.round((to-from)*axisScale(limits,'x'));
  }
  function pixels(g, limits = DEFAULTS) {
    return { width: limits.fixed_x ? assertFixedSpan(g,limits).width_px : Math.round((g.right - g.left) * axisScale(limits,'x')) + limits.border_px,
      height: Math.round((g.bottom - g.top) * axisScale(limits,'y')) + limits.border_px };
  }
  function actualFits(header, limits) {
    return header.width > 0 && header.height > 0 && header.width < 32766 && header.height < 32766 &&
      header.width <= limits.max_edge_px && header.height <= limits.max_edge_px &&
      header.width * header.height <= limits.max_pixels;
  }
  function fitAxis(observations, axis) {
    const side=axis==='x'?'width':'height';
    const points=observations.map(o=>[axis==='x'?o.geometry.right-o.geometry.left:o.geometry.bottom-o.geometry.top,o.png[side]]);
    const values=points.map(p=>p[0]);
    if (new Set(values).size<3 || Math.max(...values)-Math.min(...values)<8)
      throw fault('CALIBRATION_INSUFFICIENT_SPAN','Small controls do not vary enough on '+axis,{points},true);
    const meanX=values.reduce((a,b)=>a+b,0)/points.length;
    const meanY=points.reduce((a,b)=>a+b[1],0)/points.length;
    let numerator=0, denominator=0;
    for(const [x,y] of points){numerator+=(x-meanX)*(y-meanY);denominator+=(x-meanX)**2;}
    const freeScale=numerator/denominator, freeIntercept=meanY-freeScale*meanX;
    // Existing overflow/seam bookkeeping supports a one-pixel image edge. Validate it;
    // do not silently accept another offset and apply incompatible crop coordinates.
    const leastSquares=points.reduce((a,[x,y])=>a+x*(y-1),0)/points.reduce((a,[x])=>a+x*x,0);
    // PNG dimensions are integer-rounded. Fitting a straight line to these rounded
    // values can turn an exact 2 into 2.000038 and drift on large tiles. Intersect
    // all scale intervals compatible with round(scale*points)+1, then use the
    // lowest-denominator rational inside that interval (no assumed DPI grid).
    const lower=Math.max(...points.map(([x,y])=>(y-1-0.5)/x));
    const upper=Math.min(...points.map(([x,y])=>(y-1+0.5)/x));
    if(!(upper>lower))throw fault('CALIBRATION_MODEL_REJECTED','No common rounded-pixel scale fits all small controls.',{axis,points,lower,upper},true);
    let scale=null;
    for(let q=1;q<=512 && scale===null;q++){
      const p=Math.ceil(lower*q-1e-10),candidate=p/q;
      if(candidate>=lower-1e-12 && candidate<upper && points.every(([x,y])=>Math.round(x*candidate)+1===y))scale=candidate;
    }
    if(scale===null)scale=Math.max(lower+1e-12,Math.min(upper-1e-12,leastSquares));
    const residuals=points.map(([x,y])=>y-(Math.round(x*scale)+1));
    if (!Number.isFinite(scale) || scale<0.25 || scale>8 || Math.abs(freeIntercept-1)>1 ||
        Math.max(...residuals.map(Math.abs))>1)
      throw fault('CALIBRATION_MODEL_REJECTED','Small PNG controls do not support a stable scale plus one image-edge pixel.',
        {axis,points,scale,free_scale:freeScale,free_intercept:freeIntercept,residuals},true);
    return {scale, compatible_scale_interval:{lower_inclusive:lower,upper_exclusive:upper}, selection:'lowest-denominator compatible rational', border_px:1, free_scale:freeScale, free_intercept:freeIntercept,
      max_residual_px:Math.max(...residuals.map(Math.abs)), sample_count:points.length,
      model:'round(points * measured_scale) + 1'};
  }
  function chooseControls(candidates) {
    // Three fitting controls must independently span both axes. They can be outside
    // the final logical area, but never change the workbook or its formatting.
    let best=null, bestScore=-Infinity;
    for(let i=0;i<candidates.length;i++)for(let j=i+1;j<candidates.length;j++)for(let k=j+1;k<candidates.length;k++){
      const group=[candidates[i],candidates[j],candidates[k]];
      const xs=group.map(c=>c.geometry.right-c.geometry.left),ys=group.map(c=>c.geometry.bottom-c.geometry.top);
      const dx=Math.max(...xs)-Math.min(...xs),dy=Math.max(...ys)-Math.min(...ys);
      if(new Set(xs).size<3 || new Set(ys).size<3 || dx<8 || dy<8)continue;
      const score=Math.log(dx)+Math.log(dy);
      if(score>bestScore){best=group;bestScore=score;}
    }
    if(!best)throw fault('CALIBRATION_NO_SAFE_CONTROLS','Could not find small visible controls with different widths and heights.',
      {candidate_count:candidates.length},true);
    // A fourth independent extent when possible; otherwise repeat the middle control
    // to test native repeatability. Only the first three are used to fit the model.
    const remaining=candidates.filter(c=>!best.includes(c));
    const extra=remaining.length?remaining[Math.floor(remaining.length/2)]:best[1];
    return [...best,extra];
  }
  function fits(g, limits = DEFAULTS) {
    const p = pixels(g, limits);
    return p.width > 1 && p.height > 1 && p.width <= limits.max_edge_px && p.height <= limits.max_edge_px && p.width * p.height <= limits.max_pixels;
  }
  function split(r, g, limits = DEFAULTS) {
    const p = pixels(g, limits), cols = r.c1-r.c0, rows = r.r1-r.r0;
    if(limits.fixed_x && (p.width>limits.max_edge_px || rows<=1))
      throw fault('FIXED_WIDTH_HORIZONTAL_OPERATION','The indivisible full-width strip does not fit. No horizontal split was queued.',{rect:r,geometry:g,pixels:p},true);
    // Prefer full-width horizontal strips: this retains all possible left-hand text sources.
    const horizontal = cols > 1 && (p.width > limits.max_edge_px || rows <= 1);
    if (!horizontal && rows > 1) {
      const mid = r.r0 + Math.floor(rows / 2);
      return [{ ...r, r1: mid }, { ...r, r0: mid }];
    }
    if (cols > 1) {
      const mid = r.c0 + Math.floor(cols / 2);
      return [{ ...r, c1: mid }, { ...r, c0: mid }];
    }
    throw fault('INDIVISIBLE_OVERSIZE', 'A single cell exceeds the capture budget; no unsafe capture was queued.', { rect:r, geometry:g, pixels:p }, true);
  }
  const SHEET_LOCAL_CODES = new Set([
    'FIXED_WIDTH_SPAN_CHANGED','FIXED_WIDTH_HORIZONTAL_OPERATION','FIXED_WIDTH_PNG_MISMATCH',
    'DRAWING_BOUND_UNRESOLVED','CALIBRATION_NO_SAFE_CONTROLS','CALIBRATION_INSUFFICIENT_SPAN',
    'CALIBRATION_MODEL_REJECTED','CALIBRATION_VALIDATION_FAILED',
    'CALIBRATION_BOOTSTRAP_BUDGET','CALIBRATION_PNG_BUDGET',
    'CALIBRATION_GEOMETRY_CHANGED','PNG_GEOMETRY_MISMATCH','GEOMETRY_CHANGED',
    'INVALID_GEOMETRY','INDIVISIBLE_OVERSIZE','OVERFLOW_CONTEXT_SPLIT',
    'AXIS_GEOMETRY_INVALID','AXIS_GEOMETRY_CHANGED','AXIS_GEOMETRY_BUDGET','AXIS_DATA_MISSING',
    'CONTEXT_TOO_THIN','CONTEXT_CROP_BOUNDS','CONTEXT_RASTER_PHASE','CONTEXT_PLAN_BUDGET',
    'CONTEXT_PNG_MISMATCH','CONTEXT_REFERENCE_MISMATCH','CONTEXT_REFERENCE_BUDGET','CONTEXT_REFERENCE_UNSTABLE'
  ]);
  function sheetLocal(e) { return !!e && SHEET_LOCAL_CODES.has(e.code); }
  function controlReason(g) {
    const w=g.right-g.left,h=g.bottom-g.top;
    if(!Number.isFinite(w)||!Number.isFinite(h))return 'nonfinite';
    if(w<=0)return 'zero-visible-width';
    if(h<=0)return 'zero-visible-height';
    if(Math.round(w*8)<1)return 'subpixel-bootstrap-width';
    if(Math.round(h*8)<1)return 'subpixel-bootstrap-height';
    if(w*8+1>8192 || h*8+1>8192)return 'bootstrap-edge-budget';
    if((w*8+1)*(h*8+1)>4000000)return 'bootstrap-area-budget';
    return null;
  }
  function fallbackRectangles(rect, phase) {
    // Geometry queries only. Never unhide, resize, activate or edit cells.
    const origins = phase===1 ? [[0,1],[0,4],[0,8],[0,16],[32,1],[32,16]] :
      [[0,32],[0,64],[64,0],[64,32],[128,64],[256,128]];
    const shapes=[[1,1],[1,2],[1,4],[2,1],[4,1],[8,1],[16,1],[2,2],[4,4]];
    const unique=new Map();
    for(const [dy,dx] of origins)for(const [nr,nc] of shapes) {
      const r0=Math.min(ROWS-nr,rect.r0+dy),c0=Math.min(COLS-nc,rect.c0+dx);
      const r={r0,r1:r0+nr,c0,c1:c0+nc};unique.set(address(r),r);
    }
    return [...unique.values()];
  }
  function independentControls(candidates) {
    // A very wide visible column may permit a narrow vertical strip and a short
    // horizontal strip, but no three rectangles spanning both axes together.
    const triples=[];
    for(const axis of ['x','y']) {
      const value=c=>axis==='x'?c.geometry.right-c.geometry.left:c.geometry.bottom-c.geometry.top;
      const byValue=new Map();
      for(const c of candidates){const v=value(c),old=byValue.get(v);
        if(!old || (c.geometry.right-c.geometry.left)*(c.geometry.bottom-c.geometry.top)<
          (old.geometry.right-old.geometry.left)*(old.geometry.bottom-old.geometry.top))byValue.set(v,c);}
      const sorted=[...byValue.values()].sort((a,b)=>value(a)-value(b));
      if(sorted.length<3 || value(sorted.at(-1))-value(sorted[0])<8)
        throw fault('CALIBRATION_NO_SAFE_CONTROLS','Bounded control search did not span both axes.',{axis,candidate_count:candidates.length},true);
      triples.push(sorted[0],sorted[Math.floor(sorted.length/2)],sorted.at(-1));
    }
    const fitting=[...new Set(triples)];
    const extra=candidates.find(c=>!fitting.includes(c))||fitting[Math.floor(fitting.length/2)];
    return [...fitting,extra];
  }
  function fixedWidthControls(candidates, rect, geometry) {
    // Reuse only controls covering the entire *same* physical horizontal interval.
    // Do not promote an arbitrary narrow control if a wider requested span exists.
    const pool=candidates.filter(c=>Math.abs(c.geometry.left-geometry.left)<1e-8 &&
      Math.abs(c.geometry.right-geometry.right)<1e-8 && !controlReason(c.geometry));
    const byHeight=new Map();
    for(const c of pool) {
      const h=c.geometry.bottom-c.geometry.top;
      if(!byHeight.has(h))byHeight.set(h,{...c,rect:{...c.rect,c0:rect.c0,c1:rect.c1}});
    }
    const sorted=[...byHeight.values()].sort((a,b)=>(a.geometry.bottom-a.geometry.top)-(b.geometry.bottom-b.geometry.top));
    if(sorted.length<3 || (sorted.at(-1).geometry.bottom-sorted.at(-1).geometry.top)-
       (sorted[0].geometry.bottom-sorted[0].geometry.top)<8) return null;
    const selected=[sorted[0],sorted[Math.floor(sorted.length/2)],sorted.at(-1)];
    const holdout=sorted.find(c=>!selected.includes(c)) || selected[1];
    return [...selected,holdout];
  }
  function fitFixedWidth(observations, geometry, rect) {
    if(observations.length<3)throw fault('CALIBRATION_INSUFFICIENT_SPAN','Fixed width requires three independently sized Y observations.',{},true);
    const width=observations[0].png.width;
    for(const o of observations) {
      if(o.png.width!==width || Math.abs(o.geometry.left-geometry.left)>1e-8 || Math.abs(o.geometry.right-geometry.right)>1e-8 ||
         o.rect.c0!==rect.c0 || o.rect.c1!==rect.c1)
        throw fault('FIXED_WIDTH_PNG_MISMATCH','Repeated full-span controls did not confirm the same PNG width.',{width,observation:o},true);
    }
    return {mode:'fixed-span',scale:null,left:geometry.left,right:geometry.right,width_px:width,
      c0:rect.c0,c1:rect.c1,sample_count:observations.length,model:'observed PNG width for this exact horizontal interval',
      horizontal_operations:'full span only; no horizontal crop, split, offset or probe extrapolation'};
  }
  function selectSheetNames(catalog, requested) {
    if(requested==null)return catalog.map(s=>s.name);
    if(!Array.isArray(requested)||!requested.length||requested.some(s=>typeof s!=='string'||!s.length)||new Set(requested).size!==requested.length)
      throw fault('INVALID_SHEET_SELECTION','Expected unique exact sheet names; whitespace is significant.',{},true);
    const names=new Set(catalog.map(s=>s.name)),missing=requested.filter(s=>!names.has(s));
    if(missing.length)throw fault('SHEET_SELECTION_MISSING','Requested sheet is absent or not visible.',{missing},true);
    return catalog.filter(s=>requested.includes(s.name)).map(s=>s.name);
  }
  class Session {
    constructor(workbook, config = {}) {
      this.workbook = workbook; this.config = config;
      this.limits = { ...DEFAULTS }; // Engineering budgets, not alleged API maxima.
      const options=config.capture_options || {};
      const megapixels=Number(options.tile_megapixels || 16);
      if (![16,32,64].includes(megapixels)) throw fault('INVALID_CAPTURE_OPTIONS','Tile budget must be 16, 32 or 64 megapixels.',{megapixels},true);
      this.limits.max_pixels=megapixels*1000000;
      if(options.native_call_timeout_ms!==undefined){
        const timeout=Number(options.native_call_timeout_ms);
        if(!Number.isInteger(timeout)||timeout<10000||timeout>600000)throw fault('INVALID_CAPTURE_OPTIONS','Native-call timeout must be 10..600 seconds.',{timeout},true);
        this.limits.timeout_ms=timeout;
      }
      this.packing=options.tile_packing || 'budget';
      if (!['budget','balanced'].includes(this.packing)) throw fault('INVALID_CAPTURE_OPTIONS','Unknown tile packing.',{packing:this.packing},true);
      this.runId = config.capture_run_id || config.diagnostic_run_id || null;
      this.dead = false; this.sequence = 0; this.tileCount = 0; this.calibrations = new Map();
      this.lastCheckpointAt=-Infinity;
      this.axisGeometry=null;
      this.state = { schema:'safe-capture-v1', workbook, strategy:config.strategy, capture_options:options, tile_plans:[], sparse_overflow_checks:[],
        status:'running', geometry_diagnostics:[], sheet_continuations:[], checkpoint_policy:{interval_ms:2000,sent:0,coalesced:0}, implementation:'safe-capture-fixed-width-v6', limits:this.limits, sheets:[], attempts:[], captures:[], calibrations:[],
        environment:{ browser: typeof navigator!=='undefined'?navigator.userAgent:null,
          office: root.Office && root.Office.context && root.Office.context.diagnostics || null,
          device_pixel_ratio: typeof root.devicePixelRatio==='number'?root.devicePixelRatio:null,
          screen: root.screen?{width:root.screen.width,height:root.screen.height,availWidth:root.screen.availWidth,availHeight:root.screen.availHeight}:null,
          note:'Browser devicePixelRatio is recorded, not used as the Excel PNG scale.' } };
      this.timings = { geometry_ms:0, render_ms:0, persist_ms:0, analysis_ms:0, telemetry_ms:0, calibration_ms:0, checkpoint_ms:0, geometry_diagnostics_ms:0 };
      if(root.CaptureGeometry)this.axisGeometry=new root.CaptureGeometry.Ledger(this);
      this.state.context_policy=this.axisGeometry?'visible-cell-halo-v1':'legacy-unpadded';
    }
    async claim() {
      const client=(root.crypto && typeof root.crypto.randomUUID==='function')?root.crypto.randomUUID():`${Date.now()}-${Math.random()}`;
      const result=await this.http('/capture-claim',{workbook:this.workbook,client_id:client});
      this.state.client_id=client;
      return result.acquired === true;
    }
    async http(url, payload) {
      const controller = typeof AbortController !== 'undefined' ? new AbortController() : null;
      const timer = controller && setTimeout(() => controller.abort(), 30000);
      try {
        const res = await fetch(url, { method:'POST', headers:{ 'Content-Type':'application/json',
          ...(this.runId ? {'X-VV-Run-Id':this.runId} : {}) },
          body:JSON.stringify(payload), ...(controller ? {signal:controller.signal} : {}) });
        if (!res.ok) throw fault('HOST_HTTP_ERROR', `${url}: HTTP ${res.status}: ${(await res.text()).slice(0,12000)}`, { url }, true);
        return await res.json();
      } catch (e) { e.fatal = true; throw e; } finally { if (timer) clearTimeout(timer); }
    }
    async event(stage, detail = {}) {
      const identity=(this.state.sheet_catalog || []).find(x=>x.name===detail.sheet);
      if(identity)detail={sheet_index:identity.sheet_index,office_position:identity.office_position,...detail};
      const t = performance.now();
      await this.http('/client-event', { stage, detail:{ workbook:this.workbook, ...detail } });
      this.timings.telemetry_ms += performance.now()-t;
    }
    async checkpoint(stage, detail = {}) {
      this.state.current = { stage, ...detail };
      const now=performance.now(),policy=this.state.checkpoint_policy;
      const force=/(failed|fatal|sheet_finished|workbook_finished|stopped|continuation|diagnostic|sparse_write_pending)/.test(stage);
      // Native starts and receipts are still individually ACKed in the host journal.
      // Only redundant full-state snapshots are coalesced, at most two seconds.
      if(!force && now-this.lastCheckpointAt<policy.interval_ms){policy.coalesced++;return;}
      const started=performance.now();policy.sent++;
      try {
        const snapshot={...this.state,precomputed_wsl_rust_visual_plan:undefined};
        const text=JSON.stringify(snapshot);policy.estimated_payload_bytes=(policy.estimated_payload_bytes||0)+new TextEncoder().encode(text).length;
        await this.http('/capture-progress', snapshot);this.lastCheckpointAt=performance.now();
      }
      finally {this.timings.checkpoint_ms+=performance.now()-started;}
    }
    async office(stage, detail, action, timeoutMs = this.limits.timeout_ms) {
      if (this.dead) throw fault('NATIVE_SESSION_STOPPED', 'No further Office calls after a native timeout.', detail, true);
      await this.event(stage + '_started', detail); // ACKed and journalled BEFORE native work.
      let timer;
      const nativeStarted=performance.now();
      try {
        return await Promise.race([Excel.run(action), new Promise((_, reject) => {
          timer = setTimeout(() => {
            this.dead = true;
            reject(fault('NATIVE_TIMEOUT', `Office call timed out at ${stage}; native work is not cancelled.`, detail, true));
          }, Math.min(this.limits.timeout_ms,timeoutMs));
        })]);
      } catch (e) {
        await this.event(stage + '_failed', { ...detail, error:errorInfo(e) });
        if (this.config.break_on_error || root.VV_BREAK_ON_ERROR) { debugger; }
        throw e;
      } finally {
        clearTimeout(timer);
        const elapsed=performance.now()-nativeStarted;
        if(stage==='geometry' || stage==='axis_geometry' || stage==='context_merge_guard') this.timings.geometry_ms+=elapsed;
        else if(stage==='tile_render' || stage==='recovery_control' || stage==='calibration_render' || stage==='sparse_render' || stage==='context_reference_render') this.timings.render_ms+=elapsed;
      }
    }
    async geometries(sheet, rects, refresh = false) {
      if(this.axisGeometry)return this.axisGeometry.geometries(sheet,rects,refresh);
      const start = performance.now();
      const out = await this.office('geometry', { sheet, rect_count:rects.length,
        addresses:rects.slice(0,32).map(address) }, async context => {
        const ws = context.workbook.worksheets.getItem(sheet);
        const cells = new Map();
        function get(row, column, extra) {
          const key = `${row}:${column}`;
          if (!cells.has(key)) { const cell = ws.getRangeByIndexes(row,column,1,1); cell.load('top,left,height,width'); cells.set(key, cell); }
          return cells.get(key);
        }
        const refs = rects.map(r => ({ r, start:get(r.r0,r.c0),
          down:get(Math.min(ROWS-1,r.r1),r.c0), right:get(r.r0,Math.min(COLS-1,r.c1)) }));
        await context.sync();
        return refs.map(x => ({ left:Number(x.start.left), top:Number(x.start.top),
          right:Number(x.right.left) + (x.r.c1 === COLS ? Number(x.right.width) : 0),
          bottom:Number(x.down.top) + (x.r.r1 === ROWS ? Number(x.down.height) : 0),
          terminal_fallback: x.r.c1 === COLS || x.r.r1 === ROWS }));
      });
      // Native geometry and logging are timed separately in office()/event().
      for (let i=0;i<out.length;i++) {
        const g=out[i];
        if (![g.left,g.top,g.right,g.bottom].every(Number.isFinite) || g.right < g.left || g.bottom < g.top)
          throw fault('INVALID_GEOMETRY','Nonfinite/nonmonotone top/left geometry',{sheet,rect:rects[i],geometry:g},true);
      }
      return out;
    }
    limitsFor(sheet) {
      const calibration=this.calibrations.get(sheet);
      if(!calibration || calibration.status!=='complete')throw fault('UNCALIBRATED_CAPTURE','Sheet has no validated calibration.',{sheet},true);
      return {...this.limits, scale:null, scale_x:calibration.x.scale, scale_y:calibration.y.scale,
        border_px:1, calibration_id:calibration.id, fixed_x:calibration.x.mode==='fixed-span'?calibration.x:null};
    }
    async findCalibrationControls(sheet, rect, initialRects, record) {
      const candidates=[],seen=new Set();record.control_search=[];
      for(let phase=0;phase<3;phase++) {
        const rects=phase===0?initialRects:fallbackRectangles(rect,phase);
        const gs=await this.geometries(sheet,rects),audit=[];
        for(let i=0;i<gs.length;i++) {
          const g=gs[i],width=g.right-g.left,height=g.bottom-g.top,key=`${width}:${height}`;
          let reason=controlReason(g);
          if(!reason&&seen.has(key))reason='duplicate-visible-extent';
          audit.push({address:address(rects[i]),geometry:g,width_points:width,height_points:height,eligible:!reason,reason});
          if(!reason){seen.add(key);candidates.push({rect:rects[i],geometry:g});}
        }
        const pass={phase,method:phase===0?'original-origin':'bounded-offset-windows',candidate_count:candidates.length,examined:audit};
        record.control_search.push(pass);
        await this.event('calibration_control_search',{sheet,calibration_id:record.id,...pass});
        try {
          const selected=chooseControls(candidates);
          record.control_selection=phase===0?'original-origin':'offset-visible-controls';
          if(phase>0)await this.diagnoseGeometry(sheet,'OFFSET_CONTROLS_REQUIRED',rect);
          return selected;
        } catch(e) {if(e.code!=='CALIBRATION_NO_SAFE_CONTROLS')throw e;}
      }
      record.control_selection='independent-axis-controls';
      let selected;
      try{selected=independentControls(candidates);}
      catch(e){
        if(e.code==='CALIBRATION_NO_SAFE_CONTROLS' && e.detail?.axis==='x') {
          const extent=(await this.geometries(sheet,[rect]))[0];
          const fixed=fixedWidthControls(candidates,rect,extent);
          if(fixed) {
            record.control_selection='fixed-width-y-controls';
            record.method='fixed horizontal interval; three Y controls plus independent/repeated holdout';
            record.fixed_span={left:extent.left,right:extent.right,c0:rect.c0,c1:rect.c1};
            return fixed;
          }
        }
        if(this.config.capture_options?.diagnostic_level==='detailed' && e.code==='CALIBRATION_NO_SAFE_CONTROLS'){
          await this.diagnoseDegenerateControls(sheet,candidates,record);
        }
        throw e;
      }
      record.method='bounded visible controls; independently spanned axes plus holdout';
      await this.diagnoseGeometry(sheet,'INDEPENDENT_AXIS_CONTROLS_REQUIRED',rect);
      return selected;
    }
    async diagnoseDegenerateControls(sheet,candidates,record) {
      // Evidence only: this does NOT admit a fixed-width calibration as successful.
      const unique=(axis)=>new Set(candidates.map(x=>axis==='x'?x.geometry.right-x.geometry.left:x.geometry.bottom-x.geometry.top)).size;
      const diagnostic={schema:'vv-degenerate-control-v1',sheet,status:'started',unique_widths:unique('x'),unique_heights:unique('y'),
        observations:[],qualification:'diagnostic-only; original calibration failure retained'};
      record.degenerate_diagnostic=diagnostic;
      if(candidates.length<3){diagnostic.status='insufficient-safe-controls';return;}
      const axis=diagnostic.unique_heights>=3?'y':'x';
      const value=x=>axis==='y'?x.geometry.bottom-x.geometry.top:x.geometry.right-x.geometry.left;
      const sorted=[...new Map(candidates.map(x=>[value(x),x])).values()].sort((a,b)=>value(a)-value(b));
      if(sorted.length<3){diagnostic.status='insufficient-variable-axis';return;}
      const controls=[sorted[0],sorted[Math.floor(sorted.length/2)],sorted.at(-1),sorted[0]];
      try{
        for(let i=0;i<controls.length;i++){
          const c=controls[i],id=`${record.id}-diagnostic${i+1}`,a=address(c.rect);
          const before=(await this.geometries(sheet,[c.rect],true))[0];
          if(controlReason(before))throw fault('CALIBRATION_BOOTSTRAP_BUDGET','Diagnostic control no longer fits its bounded bootstrap budget.',{sheet,address:a,before},true);
          let base64=null;
          try{
            base64=await this.office('calibration_render',{sheet,address:a,id,diagnostic_only:true},async context=>{
              const result=context.workbook.worksheets.getItem(sheet).getRange(a).getImage();await context.sync();return result.value;
            });
            const header=png(base64);
            if(!actualFits(header,{...this.limits,max_edge_px:8192,max_pixels:4000000}))throw fault('CALIBRATION_PNG_BUDGET','Diagnostic PNG exceeded its budget.',{sheet,header},true);
            const receipt=await this.http('/image-batch',{workbook:this.workbook,batch_index:0,images:[{id,sheet,address:a,base64}]});
            if(!receipt.captures?.[0])throw fault('PERSIST_NO_RECEIPT','Diagnostic image not acknowledged.',{id},true);
            const after=(await this.geometries(sheet,[c.rect],true))[0];
            const sample={id,address:a,geometry_before:before,geometry:after,png:header,relative_png:receipt.captures[0].relative_png,
              geometry_stable:['left','top','right','bottom'].every(k=>Math.abs(before[k]-after[k])<=0.01)};
            diagnostic.observations.push(sample);await this.event('degenerate_control_sample',{sheet,...sample});
          }finally{base64=null;}
        }
        try{diagnostic.variable_axis=axis;diagnostic.variable_axis_fit=fitAxis(diagnostic.observations.slice(0,3),axis);}catch(e){diagnostic.fit_error=errorInfo(e);}
        diagnostic.status='evidence-recorded';
      }catch(e){diagnostic.status='failed';diagnostic.error=errorInfo(e);if(this.dead || e.code==='HOST_HTTP_ERROR')throw e;}
      await this.event('degenerate_control_diagnostic',diagnostic);
      await this.checkpoint('degenerate_control_diagnostic',{sheet,status:diagnostic.status});
    }
    async diagnoseGeometry(sheet, reason, rect) {
      // At most one diagnostic per reason/sheet. This is a small read-only geometry
      // audit, not a content/UsedRange scan and not a new PNG fit used to waive failure.
      const key=sheet+'|'+reason;
      const prior=this.state.geometry_diagnostics.find(x=>x.key===key);
      if(prior)return prior;
      const record={key,sheet,reason,address:address(rect),status:'started',columns:[],rows:[]};
      this.state.geometry_diagnostics.push(record);
      const started=performance.now();
      try {
        const data=await this.office('geometry_diagnostic',{sheet,reason,address:record.address},async context=>{
          const ws=context.workbook.worksheets.getItem(sheet),columns=[],rows=[];
          const colIndexes=[...new Set([0,1,2,3,4,5,6,7,8,15,16,31,rect.c0,Math.min(COLS-1,rect.c1)])];
          const rowIndexes=[...new Set([...Array.from({length:16},(_,i)=>i),rect.r0,
            Math.max(0,rect.r1-2),Math.min(ROWS-1,rect.r1-1),Math.min(ROWS-1,rect.r1)])];
          for(const c of colIndexes){const cell=ws.getRangeByIndexes(0,c,1,1);cell.load('left,width,columnHidden');columns.push({c,cell});}
          for(const r of rowIndexes)for(const c of [...new Set([rect.c0,Math.min(COLS-1,rect.c0+1)])]) {
            const cell=ws.getRangeByIndexes(r,c,1,1);cell.load('top,left,height,width,rowHidden,columnHidden');
            if(cell.format&&typeof cell.format.load==='function')cell.format.load('rowHeight');
            rows.push({r,c,cell});
          }
          await context.sync();
          return {columns:columns.map(x=>({column_1based:x.c+1,column:col(x.c),left_points:Number(x.cell.left),
              width_points:Number(x.cell.width),hidden:x.cell.columnHidden??null})),
            rows:rows.map(x=>({row_1based:x.r+1,anchor_column:col(x.c),top_points:Number(x.cell.top),
              height_points:Number(x.cell.height),width_points:Number(x.cell.width),row_hidden:x.cell.rowHidden??null,
              anchor_column_hidden:x.cell.columnHidden??null,row_height_points:x.cell.format?.rowHeight??null}))};
        },10000);
        Object.assign(record,data,{status:'complete'});
      } catch(e) {
        record.status='failed';record.error=errorInfo(e);
        // A diagnostic timeout poisons the same native session. Never continue it.
        if(this.dead || e.code==='HOST_HTTP_ERROR')throw e;
      } finally {record.duration_ms=performance.now()-started;this.timings.geometry_diagnostics_ms+=record.duration_ms;}
      await this.event('geometry_diagnostic_result',record);
      await this.checkpoint('geometry_diagnostic_finished',{sheet,reason});return record;
    }
    async continueAfterSheetFailure(sheet, error) {
      if(this.state.sparse_overflow_checks.some(c=>c.write_attempted_cells>0 && c.restoration!=='verified' && c.roundtrip_identical!==true)){this.dead=true;return false;}
      if(this.dead || (error.fatal && !sheetLocal(error)))return false;
      const record={sheet,error:errorInfo(error),scope:'worksheet',status:'checking-session'};
      this.state.sheet_continuations.push(record);
      const attempt=[...this.state.attempts].reverse().find(a=>a.sheet===sheet&&a.error);
      if(attempt && sheetLocal(error))await this.diagnoseGeometry(sheet,error.code,attempt.rect);
      if(this.dead)return false;
      try {
        await this.office('sheet_health_check',{sheet,after_error:error.code},async context=>{
          const ws=context.workbook.worksheets.getItem(sheet);ws.load('name,visibility');await context.sync();
        },10000);
        record.status='healthy-continue';
        await this.event('sheet_failure_isolated',record);
        await this.checkpoint('sheet_continuation',{sheet,error_code:error.code});return true;
      } catch(e) {record.status='stop-session';record.health_error=errorInfo(e);this.dead=true;return false;}
    }
    async ensureCalibration(sheet, rect) {
      if(this.calibrations.has(sheet))return this.calibrations.get(sheet);
      const started=performance.now();
      const record={id:`cal${String(this.state.calibrations.length+1).padStart(4,'0')}`,sheet,
        status:'running',observations:[],method:'three small fitting controls plus one independent/repeated validation control',
        bootstrap_scale_upper_bound:8,bootstrap_max_pixels:4000000};
      this.state.calibrations.push(record);
      await this.event('raster_calibration_started',{sheet,id:record.id,method:record.method});
      await this.checkpoint('raster_calibration_started',{sheet,id:record.id});
      try {
        const r0=Math.min(rect.r0,ROWS-32),c0=Math.min(rect.c0,COLS-16);
        const shapes=[[1,1],[2,2],[4,4],[8,8],[16,16],[32,16],[2,4],[4,8],[8,2],[16,1],[1,8],[32,1],[4,2],[8,4]];
        const rects=shapes.map(([nr,nc])=>({r0,r1:r0+nr,c0,c1:c0+nc}));
        const controls=await this.findCalibrationControls(sheet,rect,rects,record);
        const fittingCount=controls.length-1;
        record.fitting_count=fittingCount;
        for(let i=0;i<controls.length;i++){
          const control=controls[i],a=address(control.rect),id=`${record.id}-${i+1}`;
          const before=(await this.geometries(sheet,[control.rect],true))[0];
          if(!fits(before,{...this.limits,scale:8,scale_x:8,scale_y:8,max_edge_px:8192,max_pixels:4000000}))
            throw fault('CALIBRATION_BOOTSTRAP_BUDGET','Calibration geometry changed beyond its small bootstrap budget.',{sheet,address:a,before},true);
          const sample={id,address:a,role:i<fittingCount?'fit':'validation',rect:control.rect,geometry_before:before,status:'started'};
          record.observations.push(sample);await this.checkpoint('calibration_render_started',{sheet,address:a,id});
          let base64=null;
          try {
            base64=await this.office('calibration_render',{sheet,address:a,id},async context=>{
              const result=context.workbook.worksheets.getItem(sheet).getRange(a).getImage();
              if(typeof context.trace==='function')context.trace(`calibration ${JSON.stringify(sheet)} ${a} ${id}`);
              await context.sync();return result.value;
            });
            sample.png=png(base64);
            const t=performance.now(),response=await this.http('/image-batch',{workbook:this.workbook,batch_index:0,
              images:[{id,sheet,address:a,base64}]});this.timings.persist_ms+=performance.now()-t;
            if(!response.captures || !response.captures[0])throw fault('PERSIST_NO_RECEIPT','Calibration PNG was not acknowledged.',{id},true);
            sample.relative_png=response.captures[0].relative_png;
            const after=(await this.geometries(sheet,[control.rect],true))[0];sample.geometry=after;
            if(['left','top','right','bottom'].some(k=>Math.abs(after[k]-before[k])>0.01))
              throw fault('CALIBRATION_GEOMETRY_CHANGED','Small control changed geometry while rendering.',{sheet,before,after},true);
            if(!actualFits(sample.png,{...this.limits,max_edge_px:8192,max_pixels:4000000}))
              throw fault('CALIBRATION_PNG_BUDGET','Calibration returned an oversized/capped PNG; it must not teach a scale.',{sheet,png:sample.png},true);
            sample.status='complete';
            await this.event('raster_calibration_sample',{sheet,...sample});
          } finally {base64=null;}
          if(i===fittingCount-1){
            record.x=record.fixed_span?fitFixedWidth(record.observations,record.fixed_span,rect):fitAxis(record.observations,'x');
            record.y=fitAxis(record.observations,'y');
          }
          if(i===fittingCount){
            if(record.fixed_span)record.x=fitFixedWidth(record.observations,record.fixed_span,rect);
            const prediction=pixels(sample.geometry,{...this.limits,scale_x:record.x.scale,scale_y:record.y.scale,
              fixed_x:record.fixed_span?record.x:null});
            sample.predicted=prediction;
            if(Math.abs(sample.png.width-prediction.width)>(record.fixed_span?0:1) || Math.abs(sample.png.height-prediction.height)>(record.fixed_span?0:1))
              throw fault('CALIBRATION_VALIDATION_FAILED','Independent small control did not confirm the fitted scale.',{sheet,sample},true);
          }
        }
        record.status='complete';record.duration_ms=performance.now()-started;
        this.calibrations.set(sheet,record);
        await this.event('raster_calibrated',{sheet,id:record.id,scale_x:record.x.scale,scale_y:record.y.scale,border_px:1,
          calibration_mode:record.fixed_span?'fixed-width':'two-axis',fixed_x:record.fixed_span?record.x:null,
          validation_count:record.observations.length,effective_dpi_x:record.x.scale===null?null:record.x.scale*72,effective_dpi_y:record.y.scale*72,
          note:'Effective DPI is calculated from measured scale, not a Windows monitor-DPI measurement.'});
        await this.checkpoint('raster_calibrated',{sheet,id:record.id});return record;
      } catch(e){
        e.fatal=true;record.status='failed';record.error=errorInfo(e);
        if(!this.dead && sheetLocal(e))await this.diagnoseGeometry(sheet,e.code,rect);
        await this.event('raster_calibration_failed',{sheet,id:record.id,error:record.error});
        await this.checkpoint('raster_calibration_failed',{sheet,id:record.id});throw e;
      } finally {this.timings.calibration_ms+=performance.now()-started;}
    }
    async planBalanced(sheet, rect) {
      const initial=(await this.geometries(sheet,[rect]))[0];
      if(initial.right===initial.left || initial.bottom===initial.top){
        await this.event('invisible_tile',{sheet,address:address(rect),rect,geometry:initial});
        return [{rect,geometry:initial,invisible:true}];
      }
      await this.ensureCalibration(sheet,rect);
      const limits=this.limitsFor(sheet);
      let pending = [rect], final = [];
      // Breadth-first reads batch candidate boundary cells. No row-by-row scan.
      for (let depth=0; pending.length; depth++) {
        if (depth > limits.max_depth || final.length + pending.length > limits.max_tiles)
          throw fault('TILE_PLAN_BUDGET','Tile planning budget exceeded; bounds may be incorrect.',{sheet,rect,depth},true);
        const next=[];
        for (let i=0;i<pending.length;i+=128) {
          const batch=pending.slice(i,i+128), gs=await this.geometries(sheet,batch);
          for (let j=0;j<batch.length;j++) {
            const r=batch[j], g=gs[j];
            if (g.right === g.left || g.bottom === g.top) {
              await this.event('invisible_tile', {sheet, address:address(r), rect:r, geometry:g});
              final.push({rect:r,geometry:g,invisible:true});
            } else if (fits(g,limits)) final.push({rect:r,geometry:g});
            else next.push(...split(r,g,limits));
          }
        }
        pending=next;
      }
      final.sort((a,b)=>a.rect.r0-b.rect.r0 || a.rect.c0-b.rect.c0);
      const record={sheet, address:address(rect), method:'Balanced subdivision', packing:'balanced', tile_count:final.length,
        full_predicted:pixels(initial,limits),allowed_height_px:Math.min(limits.max_edge_px,Math.floor(limits.max_pixels/pixels(initial,limits).width)),
        horizontal_split:final.some(t=>t.rect.c0 !== rect.c0 || t.rect.c1 !== rect.c1),
        limits,tiles:final.map(t=>({rect:t.rect, geometry:t.geometry, predicted:pixels(t.geometry,limits), invisible:!!t.invisible}))};
      this.state.tile_plans.push(record);await this.event('tile_plan',record);
      return final;
    }
    // Measured full-width packing; use the largest fitting row boundary instead of
    // rounding the tile count up to a power of two. No cell-content scan is performed.
    async plan(sheet, rect, parent = rect) {
      if(this.axisGeometry)return root.CaptureGeometry.plan(this,sheet,rect,parent);
      if (this.packing === 'balanced') return this.planBalanced(sheet, rect);
      const first=(await this.geometries(sheet,[rect]))[0];
      if(first.right===first.left || first.bottom===first.top) return [{rect,geometry:first,invisible:true}];
      await this.ensureCalibration(sheet,rect);
      const limits=this.limitsFor(sheet), fullPixels=pixels(first,limits);
      if(fullPixels.width>limits.max_edge_px || rect.r1-rect.r0<=1) return this.planBalanced(sheet,rect);
      const cache=new Map([[rect.r0,first.top],[rect.r1,first.bottom]]);
      const top=async index=>{
        if(!cache.has(index)) {
          const rr={r0:index===ROWS?ROWS-1:index,r1:index===ROWS?ROWS:index+1,c0:rect.c0,c1:rect.c0+1};
          const g=(await this.geometries(sheet,[rr]))[0];cache.set(index,index===ROWS?g.bottom:g.top);
        }
        return cache.get(index);
      };
      const maxHeight=Math.min(limits.max_edge_px,Math.floor(limits.max_pixels/fullPixels.width));
      const targetPoints=Math.max(0,(maxHeight-limits.border_px-1)/limits.scale_y);
      const final=[];let start=rect.r0;
      while(start<rect.r1) {
        if(final.length>=limits.max_tiles)throw fault('TILE_PLAN_BUDGET','Measured packing exceeded tile count.',{sheet,rect},true);
        const y0=await top(start), yEnd=await top(rect.r1);
        const geom=end=>({left:first.left,right:first.right,top:y0,bottom:cache.get(end),terminal_fallback:end===ROWS});
        let good=start,bad=rect.r1+1,probe=rect.r1;
        if(!fits(geom(rect.r1),limits)) {
          probe=Math.max(start+1,Math.min(rect.r1, start+Math.floor((rect.r1-start)*targetPoints/Math.max(0.001,yEnd-y0))));
        }
        let tries=0;
        while(true) {
          if(++tries>64)throw fault('TILE_SEARCH_BUDGET','Could not bracket a stable row boundary.',{sheet,start},true);
          await top(probe);
          const g=geom(probe),flat=g.bottom===g.top;
          if(flat || fits(g,limits)) {
            good=probe;
            if(good===rect.r1)break;
            // Certify local maximality. Hidden rows have zero height and may be included.
            if(probe+1<bad) {
              await top(probe+1);
              if(fits(geom(probe+1),limits) || geom(probe+1).bottom===y0) {
                good=probe+1;
                if(good===rect.r1)break;
                const perRow=(cache.get(good)-y0)/(good-start);
                const jump=Math.max(1,Math.floor((targetPoints-(cache.get(good)-y0))/Math.max(perRow,0.001)));
                probe=Math.min(bad-1,rect.r1,Math.max(good+1,good+jump));continue;
              }
              bad=probe+1;
            }
          } else bad=probe;
          if(bad-good<=1)break;
          probe=Math.floor((good+bad)/2);
        }
        if(good===start) {
          // A very tall single row needs horizontal splitting or a terminal failure.
          const one={...rect,r0:start,r1:start+1};
          final.push(...await this.planBalanced(sheet,one));start++;continue;
        }
        const r={...rect,r0:start,r1:good},g=geom(good),p=pixels(g,limits);
        final.push({rect:r,geometry:g,invisible:g.bottom===g.top,
          packing:{method:'measured-row-boundary',limiting_budget:maxHeight===limits.max_edge_px?'edge':'pixel-area',
            allowed_height_px:maxHeight,predicted_pixels:p.width*p.height,area_utilization:p.width*p.height/limits.max_pixels,
            next_row_exceeds_budget:good<rect.r1}});
        start=good;
      }
      const record={sheet,address:address(rect),method:'Budget-aware row boundaries',packing:'budget',limits,allowed_height_px:maxHeight,tile_count:final.length,
        horizontal_split:final.some(t=>t.rect.c0!==rect.c0||t.rect.c1!==rect.c1),max_pixels:limits.max_pixels,
        max_edge_px:limits.max_edge_px,full_predicted:fullPixels,allowed_height_at_full_width_px:maxHeight,
        tiles:final.map(t=>({rect:t.rect,predicted:pixels(t.geometry,limits),packing:t.packing}))};
      this.state.tile_plans.push(record);await this.event('tile_plan',record);return final;
    }
    async columnWidths(sheet, end) {
      const rects = [];
      for (let c=0;c<=end;c++) rects.push({r0:0,r1:1,c0:c,c1:c+1});
      const widths = [];
      for (let i=0;i<rects.length;i+=256) {
        const gs=await this.geometries(sheet,rects.slice(i,i+256));
        widths.push(...gs.map(g=>g.right-g.left));
      }
      return widths;
    }
    async capture(sheet, tile, logicalId, onImage, depth = 0) {
      if (tile.invisible) return [];
      const limits=this.limitsFor(sheet);
      if (++this.sequence > limits.max_attempts || ++this.tileCount > limits.max_tiles*2)
        throw fault('CAPTURE_BUDGET','Capture/recovery budget exceeded',{sheet},true);
      const id=`${logicalId}-t${String(this.sequence).padStart(6,'0')}`;
      const rect=tile.rect, captureRect=tile.capture_rect || rect, parent=tile.parent_rect || rect;
      if(limits.fixed_x && (captureRect.c0!==limits.fixed_x.c0 || captureRect.c1!==limits.fixed_x.c1))
        throw fault('FIXED_WIDTH_HORIZONTAL_OPERATION','Capture columns differ from the calibrated fixed interval.',{sheet,captureRect,fixed_x:limits.fixed_x},true);
      const a=address(captureRect), before=(await this.geometries(sheet,[captureRect],true))[0];
      if (!fits(before,limits)) {
        if (depth>=limits.max_depth) throw fault('TILE_DEPTH','Capture split depth exceeded',{sheet,a},true);
        const children=await this.plan(sheet,rect,parent); const results=[];
        for (const child of children) results.push(...await this.capture(sheet,child,logicalId,onImage,depth+1));
        return results;
      }
      const identity=(this.state.sheet_catalog || []).find(x=>x.name===sheet);
      const attempt={id,sheet,sheet_index:identity ? identity.sheet_index : null,
        address:a,rect,capture_rect:captureRect,parent_rect:parent,guards:tile.guards||null,
        geometry_before:before,predicted:pixels(before,limits),status:'started'};
      this.state.attempts.push(attempt);
      await this.checkpoint('tile_render_started',{sheet,address:a,id});
      let base64=null, saved=null, failureStage='render';
      try {
        const renderBefore=this.timings.render_ms;
        base64=await this.office('tile_render',{sheet,address:a,id,predicted:attempt.predicted},async context=>{
          const image=context.workbook.worksheets.getItem(sheet).getRange(a).getImage();
          if (typeof context.trace === 'function') context.trace(`safe getImage sheet=${JSON.stringify(sheet)} range=${a} tile=${id}`);
          await context.sync(); return image.value;
        });
        attempt.render_ms=this.timings.render_ms-renderBefore;
        failureStage='payload-validation';
        const header=png(base64); attempt.png=header;
        await this.event('tile_png_received',{sheet,address:a,id,...header,render_ms:attempt.render_ms});
        // Save the original BEFORE browser decoding/analysis. Its bytes are never re-encoded.
        failureStage='persistence';
        const started=performance.now();
        const response=await this.http('/image-batch',{workbook:this.workbook,batch_index:0,
          images:[{id,sheet,address:a,base64}]});
        this.timings.persist_ms+=performance.now()-started;
        saved=response.captures && response.captures[0];
        if (!saved) throw fault('PERSIST_NO_RECEIPT','Host did not return saved capture metadata',{id},true);
        attempt.relative_png=saved.relative_png;
        failureStage='post-capture-validation';
        const after=(await this.geometries(sheet,[captureRect],true))[0]; attempt.geometry_after=after;
        const expected=pixels(after,limits);
        if(limits.fixed_x && header.width!==expected.width)
          throw fault('FIXED_WIDTH_PNG_MISMATCH','Full-span PNG width changed; no crop or rescaling was accepted.',{header,expected,fixed_x:limits.fixed_x},true);
        const changed=['left','top','right','bottom'].some(k=>Math.abs(before[k]-after[k])>0.01);
        if (changed) throw fault('GEOMETRY_CHANGED','Geometry changed while capturing; no mixed-layout success is reported.',{before,after,header},true);
        if (header.width>=32766 || header.height>=32766)
          throw fault('PNG_DIMENSION_CAP','PNG reached the observed clipping dimension; smaller captures are required.',{header,expected,after});
        if (Math.abs(header.width-expected.width)>limits.tolerance_px || Math.abs(header.height-expected.height)>limits.tolerance_px)
          throw fault('PNG_GEOMETRY_MISMATCH','PNG differs from the measured calibration. Stop instead of splitting down to single cells.',
            {header,expected,after,calibration_id:limits.calibration_id,scale_x:limits.scale_x,scale_y:limits.scale_y,
              observed_scale_x:(header.width-1)/(after.right-after.left),observed_scale_y:(header.height-1)/(after.bottom-after.top)},true);
        if (!actualFits(header,limits) || !fits(after,limits))
          throw fault('PNG_ACTUAL_BUDGET','Actual PNG exceeds a capture budget; no Canvas decode was attempted.',{header,expected,after});
        const core=(await this.geometries(sheet,[rect]))[0],envelope=(await this.geometries(sheet,[parent]))[0];
        const coreBox=this.axisGeometry ? root.CaptureGeometry.cropBox(core,after,envelope,limits,header) : {x:0,y:0,width:header.width,height:header.height};
        if(this.axisGeometry && (header.width!==expected.width || header.height!==expected.height))
          throw fault('CONTEXT_PNG_MISMATCH','Exact context crop requires the measured image dimensions; no tolerance or resizing is used.',{header,expected,sheet,address:a},true);
        const capture={...saved,sheet_index:attempt.sheet_index,rect,geometry:core,logical_id:logicalId,packing:tile.packing || {method:this.axisGeometry?'contextual-row-packing':'balanced-subdivision'},
          capture_rect:captureRect,capture_geometry:after,parent_rect:parent,core_box_px:coreBox,guards:tile.guards||null,
          context_policy:this.axisGeometry?'visible-cell-halo-v1':'legacy-unpadded',merge_context:tile.merge_context||[],
          raster_profile:{calibration_id:limits.calibration_id,scale_x:limits.scale_x,scale_y:limits.scale_y,border_px:1,fixed_x:limits.fixed_x||null},
          content_box_px:{x:0,y:0,width:header.width,height:header.height},
          seam_extra_pixel:limits.border_px};
        failureStage='analysis';
        const analysisStart=performance.now(); let detail=null;
        try {detail=onImage ? await onImage(base64,capture) : null;}
        finally {this.timings.analysis_ms+=performance.now()-analysisStart;}
        // Failed analysis time is included too; it is not native rendering time.
        failureStage='publication';
        attempt.status='complete'; capture.analysis=detail;
        this.state.captures.push(capture);
        await this.checkpoint('tile_complete',{sheet,address:a,id});
        return [capture];
      } catch (e) {
        attempt.status=e.fatal?'fatal':'failed'; attempt.error=errorInfo(e); attempt.failure_stage=failureStage;
        await this.event('tile_failed',{sheet,address:a,id,raw_png:saved && saved.relative_png,failure_stage:failureStage,error:errorInfo(e)});
        await this.checkpoint('tile_failed',{sheet,address:a,id});
        if (this.config.break_on_error || root.VV_BREAK_ON_ERROR) { debugger; }
        // A saved native PNG must not be recaptured because analysis or persistence
        // failed. Only native rendering/payload failure and explicit size guards
        // can be made smaller. Otherwise retain the evidence and propagate.
        const sizeFailure=['PNG_DIMENSION_CAP','PNG_ACTUAL_BUDGET'].includes(e.code);
        const renderFailure=failureStage==='render' || failureStage==='payload-validation';
        const retryable = !e.fatal && (renderFailure || sizeFailure) && depth<limits.max_depth && (rect.r1-rect.r0>1 || rect.c1-rect.c0>1);
        if (!retryable) throw e;
        // Do not repeat the same oversized request. A successful small control precedes splitting.
        let control;
        try { control=await this.office('recovery_control',{sheet,after:id},async context=>{
          const result=context.workbook.worksheets.getItem(sheet).getRange(this.calibrations.get(sheet).observations[0].address).getImage();
          await context.sync(); return result.value;
        }); } catch(controlError) {this.dead=true;controlError.fatal=true;throw controlError;}
        try { png(control); } catch(controlError) {this.dead=true;controlError.fatal=true;throw controlError;}
        await this.event('tile_recovery_split',{sheet,address:a,id});
        const out=[];
        for (const r of split(rect,before,limits)) {
          for (const child of await this.plan(sheet,r,parent)) out.push(...await this.capture(sheet,child,logicalId,onImage,depth+1));
        }
        return out;
      } finally { base64=null; }
    }
    async capturePlan(sheet, tiles, logicalId, onImage) {
      if(this.axisGeometry && this.config.capture_options?.context_validation==='first-seam' && root.ContextValidation)
        await root.ContextValidation.verify(this,sheet,tiles,logicalId);
      const out=[];
      for (const tile of tiles) out.push(...await this.capture(sheet,tile,logicalId,onImage));
      return out;
    }
  }
  const api={DEFAULTS,Session,parse,address,png,pixels,fits,split,fault,errorInfo,axisScale,actualFits,fitAxis,chooseControls,independentControls,controlReason,fallbackRectangles,sheetLocal,selectSheetNames,assertFixedSpan,xOffset,fixedWidthControls,fitFixedWidth};
  root.SafeCapture=api;
  if(typeof module!=='undefined' && module.exports) module.exports=api;
}(typeof window!=='undefined'?window:globalThis));
