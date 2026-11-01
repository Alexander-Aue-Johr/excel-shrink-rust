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
    const runs = [];
    for (const span of command.spans) {
      const selected = await CaptureGeometry.readHeightRuns(
        measuredOffice, command.sheet, span.end - 1, span.start - 1);
      runs.push(...selected);
    }
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
          const deadline = Date.now() + 120000;
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
      const range = context.workbook.worksheets.getItem(command.sheet).getRange(command.range);
      if (command.kind === 'capture') {
        const image = range.getImage();
        await context.sync();
        return {image_base64: image.value};
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
