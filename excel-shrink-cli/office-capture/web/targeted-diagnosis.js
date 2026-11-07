/* Targeted comparisons operate on the same live workbook as their height checks. */
(function (root) {
  'use strict';

  async function measureHeights(command, office) {
    const started = performance.now();
    const measuredOffice = async action => {
      if (performance.now() - started > command.budget_ms) {
        throw new Error('Targeted height measurement deadline exceeded; coverage is incomplete');
      }
      return office(action);
    };
    const runs = await CaptureGeometry.readHeightSpans(measuredOffice, command.sheet, command.spans);
    return {runs, elapsed_ms: performance.now() - started, complete: true};
  }

  /** Queue geometry together, then render only nonempty ranges in one image sync. */
  async function captureRanges(context, command) {
    const requests = command.kind === 'capture_many' ? command.ranges : [command];
    const megapixels = command.batch_megapixels ?? 32;
    if (!Number.isInteger(megapixels) || megapixels < 32 || megapixels > 256) {
      throw new Error('Capture package budget must be 32..256 MP');
    }
    if (!Array.isArray(requests) || requests.length < 1 || requests.length > 64 ||
        requests.some(item => typeof item.range !== 'string' || !item.range)) {
      throw new Error('Capture batch requires 1..64 ranges');
    }
    const worksheet = context.workbook.worksheets.getItem(command.sheet);
    worksheet.showGridlines = false;
    const loaded = requests.map(request => {
      const range = worksheet.getRange(request.range);
      range.load('top,left,width,height');
      const core = request.comparison_core_range ? worksheet.getRange(request.comparison_core_range) : null;
      if (core) core.load('top,left,width,height');
      return {request, range, core};
    });
    await context.sync();
    let pixels = 0;
    const captures = loaded.map(({request, range, core}) => {
      const geometry = {top: range.top, left: range.left, width: range.width, height: range.height};
      if (core) geometry.comparison_core = {top: core.top, left: core.left, width: core.width, height: core.height};
      if (![geometry.top, geometry.left, geometry.width, geometry.height]
          .every(value => Number.isFinite(value) && value >= 0)) {
        throw new Error('Invalid measured capture geometry');
      }
      const empty = range.width === 0 || range.height === 0;
      if (!empty) pixels += (Math.ceil(range.width * 2) + 2) * (Math.ceil(range.height * 2) + 2);
      return {request, range, geometry, empty};
    });
    // Individual exports keep their native image limits. This allowance bounds
    // the sum of independently rendered images waiting in one synchronization.
    if (requests.length > 1 && pixels > megapixels * 1000000) throw new Error('Capture batch exceeds the combined pixel budget');
    const images = captures.map(item => item.empty ? null : item.range.getImage());
    if (images.some(Boolean)) await context.sync();
    return captures.map((item, index) => item.empty
      ? {range: item.request.range, non_rendering: true, reason: 'zero-area-range', geometry: item.geometry}
      : {range: item.request.range, image_base64: images[index].value, geometry: item.geometry});
  }

  /** Execute one bounded operation; save/rebuild never replace a missing response with success. */
  async function run(command, office, reportProgress = () => {}) {
    if (command.kind === 'heights') return measureHeights(command, office);
    return office(async context => {
      if (command.kind === 'measure_batch') {
        if (!Array.isArray(command.ranges) || command.ranges.length < 1 || command.ranges.length > 4096 ||
            command.ranges.some(address => typeof address !== 'string' || !address)) {
          throw new Error('Geometry batch requires 1..4096 range addresses');
        }
        const sheet = context.workbook.worksheets.getItem(command.sheet);
        const ranges = command.ranges.map(address => {
          const range = sheet.getRange(address);
          range.load('top,left,width,height');
          return range;
        });
        await context.sync();
        return {ranges: ranges.map((range, index) => ({
          range: command.ranges[index],
          top: range.top, left: range.left, width: range.width, height: range.height
        }))};
      }
      if (command.kind === 'save' || command.kind === 'recalculate') {
        if (command.kind === 'recalculate') {
          reportProgress('rebuild_dispatch');
          context.workbook.application.calculate(Excel.CalculationType.fullRebuild);
          await context.sync();
          reportProgress('rebuild_waiting_for_calculation');
          const deadline = Date.now() + Math.max(120000, (command.timeout_seconds || 120) * 1000);
          let pendingSince = null;
          let lastStateObservedAt = null;
          while (true) {
            context.workbook.application.load('calculationState');
            await context.sync();
            const state = context.workbook.application.calculationState;
            if (state === Excel.CalculationState.done) break;
            const observedAt = Date.now();
            if (state === Excel.CalculationState.pending) {
              // Opening another workbook can block Office.js; that gap is not observed idle time.
              if (pendingSince === null || lastStateObservedAt === null ||
                  observedAt - lastStateObservedAt > 2000) pendingSince = observedAt;
              if (command.external_link_parts > 0 && command.linked_pending_timeout_ms > 0 &&
                  Date.now() - pendingSince >= command.linked_pending_timeout_ms) {
                const error = new Error(`Rebuild remained Pending for ${command.linked_pending_timeout_ms / 1000} seconds with ${command.external_link_parts} external-link parts; workbook left unverified`);
                error.code = 'RebuildPendingExternalLinks';
                throw error;
              }
            } else {
              pendingSince = null;
            }
            lastStateObservedAt = observedAt;
            if (Date.now() >= deadline) throw new Error('Targeted rebuild did not finish');
            await new Promise(resolve => setTimeout(resolve, 25));
          }
        }
        reportProgress('saving_workbook');
        context.workbook.save(Excel.SaveBehavior.save);
        await context.sync();
        reportProgress('workbook_saved');
        return {saved: true};
      }
      if (command.kind === 'charts') {
        const sheets = context.workbook.worksheets;
        sheets.load('items/name');
        await context.sync();
        for (const sheet of sheets.items) sheet.charts.load('items/name,items/top,items/left,items/width,items/height');
        await context.sync();
        const charts = sheets.items.flatMap(sheet => sheet.charts.items.map((chart, index) => ({
          sheet: sheet.name,
          name: chart.name,
          index,
          geometry: {top: chart.top, left: chart.left, width: chart.width, height: chart.height},
        })));
        return {charts};
      }
      if (command.kind === 'capture_chart') {
        const charts = context.workbook.worksheets.getItem(command.sheet).charts;
        const indexed = command.chart_index !== undefined && command.chart_index !== null;
        if (indexed && (!Number.isInteger(command.chart_index) || command.chart_index < 0)) {
          throw new Error('Invalid chart collection index');
        }
        const chart = indexed ? charts.getItemAt(command.chart_index) : charts.getItem(command.chart);
        chart.load('name,top,left,width,height');
        await context.sync();
        if (indexed && chart.name !== command.chart) {
          throw new Error('Chart collection order changed since enumeration');
        }
        const geometry = {top: chart.top, left: chart.left, width: chart.width, height: chart.height};
        if (chart.width === 0 || chart.height === 0) {
          return {non_rendering: true, reason: 'zero-area-chart', geometry};
        }
        const image = chart.getImage();
        await context.sync();
        return {
          image_base64: image.value,
          geometry
        };
      }
      if (command.kind === 'capture' || command.kind === 'capture_many') {
        const images = await captureRanges(context, command);
        return command.kind === 'capture' ? images[0] : {images};
      }
      const range = context.workbook.worksheets.getItem(command.sheet).getRange(command.range);
      if (command.kind === 'measure') {
        range.load('top,left,width,height');
        await context.sync();
        return {top: range.top, left: range.left, width: range.width, height: range.height};
      }
      throw new Error(`Unknown targeted operation: ${command.kind}`);
    });
  }

  root.TargetedDiagnosis = {run};
}(typeof window !== 'undefined' ? window : globalThis));
