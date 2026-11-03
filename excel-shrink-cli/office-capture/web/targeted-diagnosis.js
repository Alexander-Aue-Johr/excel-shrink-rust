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
  async function run(command, office) {
    if (command.kind === 'heights') return measureHeights(command, office);
    return office(async context => {
      if (command.kind === 'save' || command.kind === 'recalculate') {
        if (command.kind === 'recalculate') {
          context.workbook.application.calculate(Excel.CalculationType.fullRebuild);
          await context.sync();
          const deadline = Date.now() + Math.max(120000, (command.timeout_seconds || 120) * 1000);
          while (true) {
            context.workbook.application.load('calculationState');
            await context.sync();
            if (context.workbook.application.calculationState === Excel.CalculationState.done) break;
            if (Date.now() >= deadline) throw new Error('Targeted rebuild did not finish');
            await new Promise(resolve => setTimeout(resolve, 25));
          }
        }
        context.workbook.save(Excel.SaveBehavior.save);
        await context.sync();
        return {saved: true};
      }
      if (command.kind === 'charts') {
        const sheets = context.workbook.worksheets;
        sheets.load('items/name');
        await context.sync();
        for (const sheet of sheets.items) sheet.charts.load('items/name');
        await context.sync();
        const charts = sheets.items.flatMap(sheet => sheet.charts.items.map(chart => ({
          sheet: sheet.name,
          name: chart.name,
        })));
        return {charts};
      }
      if (command.kind === 'capture_chart') {
        const chart = context.workbook.worksheets.getItem(command.sheet).charts.getItem(command.chart);
        chart.load('top,left,width,height');
        await context.sync();
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
        range.load('top,left,width,height');
        await context.sync();
        const geometry = {top: range.top, left: range.left, width: range.width, height: range.height};
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
