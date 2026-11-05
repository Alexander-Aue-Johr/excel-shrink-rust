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

  /** Execute one bounded operation; save/rebuild never replace a missing response with success. */
  async function run(command, office, reportProgress = () => {}) {
    if (command.kind === 'heights') return measureHeights(command, office);
    return office(async context => {
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
        for (const sheet of sheets.items) sheet.charts.load('items/name');
        await context.sync();
        const charts = sheets.items.flatMap(sheet => sheet.charts.items.map((chart, index) => ({
          sheet: sheet.name,
          name: chart.name,
          index,
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
      const range = context.workbook.worksheets.getItem(command.sheet).getRange(command.range);
      if (command.kind === 'capture') {
        const worksheet = context.workbook.worksheets.getItem(command.sheet);
        worksheet.showGridlines = false;
        range.load('top,left,width,height');
        const core = command.comparison_core_range
          ? worksheet.getRange(command.comparison_core_range) : null;
        if (core) core.load('top,left,width,height');
        await context.sync();
        const geometry = {top: range.top, left: range.left, width: range.width, height: range.height};
        if (core) geometry.comparison_core = {
          top: core.top, left: core.left, width: core.width, height: core.height
        };
        if (![geometry.top, geometry.left, geometry.width, geometry.height]
          .every(value => Number.isFinite(value) && value >= 0)) {
          throw new Error('Invalid measured capture geometry');
        }
        if (range.width === 0 || range.height === 0) {
          return {non_rendering: true, reason: 'zero-area-range', geometry};
        }
        const image = range.getImage();
        await context.sync();
        return {
          image_base64: image.value,
          geometry
        };
      }
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
