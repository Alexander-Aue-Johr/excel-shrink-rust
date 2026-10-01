/* global Excel, ExperimentUi, ExperimentEvents */
'use strict';

(async function () {
  function fixed(value) { return Number(Number(value).toFixed(6)); }

  async function measure() {
    return Excel.run(async (context) => {
      const workbook = context.workbook;
      const sheets = workbook.worksheets;
      workbook.load('name');
      sheets.load('items/name');
      await context.sync();

      ExperimentUi.setWorkbook(workbook.name);
      ExperimentUi.setPhase('Loading shape geometry');
      ExperimentUi.setTotalImages(sheets.items.length);

      const entries = [];
      for (const sheet of sheets.items) {
        const shapes = sheet.shapes;
        shapes.load(
          'items/left,items/top,items/width,items/height,' +
          'items/rotation,items/visible'
        );
        entries.push({ sheet, shapes });
      }

      const started = performance.now();
      await context.sync();

      const result = {};
      let completed = 0;

      for (const entry of entries) {
        let maxRight = 0;
        let maxBottom = 0;

        for (const shape of entry.shapes.items) {
          if (!shape.visible) continue;
          const angle = Number(shape.rotation) * Math.PI / 180.0;
          const width = Number(shape.width);
          const height = Number(shape.height);
          const rotatedWidth =
            Math.abs(width * Math.cos(angle)) +
            Math.abs(height * Math.sin(angle));
          const rotatedHeight =
            Math.abs(width * Math.sin(angle)) +
            Math.abs(height * Math.cos(angle));

          maxRight = Math.max(
            maxRight,
            Number(shape.left) + width / 2 + rotatedWidth / 2
          );
          maxBottom = Math.max(
            maxBottom,
            Number(shape.top) + height / 2 + rotatedHeight / 2
          );
        }

        result[entry.sheet.name] = {
          right: fixed(maxRight),
          bottom: fixed(maxBottom)
        };

        completed += 1;
        ExperimentUi.updateProgress(completed, entries.length);
      }

      return {
        workbook: workbook.name,
        sheets: result,
        geometry_total_ms: fixed(performance.now() - started),
        excel_api_requirement_set: '1.11'
      };
    });
  }

  try {
    await ExperimentEvents.post('measurement_started', { mode: 'shape-bounds' });
    const payload = await measure();
    const response = await fetch('/result', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload)
    });
    if (!response.ok) throw new Error(await response.text());
    await ExperimentEvents.post('result_submitted', { workbook: payload.workbook });
    ExperimentUi.setPhase('Complete · waiting for host to close workbook');
  } catch (error) {
    const detail = error && error.stack ? error.stack : String(error);
    ExperimentUi.fail(detail);
    await ExperimentEvents.post('error', { message: detail });
  }
}());
