/* global Office, Excel */
'use strict';

async function postClientEvent(stage, detail = null) {
  try {
    await fetch('/client-event', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        stage,
        detail,
        href: window.location.href,
        user_agent: navigator.userAgent
      })
    });
  } catch (_) {
  }
}

function fixed(value) {
  return Number(Number(value).toFixed(6));
}

function setStatus(message) {
  const element = document.getElementById('status');
  if (element) element.textContent = message;
}

async function measureWorkbook() {
  return Excel.run(async (context) => {
    const workbook = context.workbook;
    const worksheets = workbook.worksheets;

    workbook.load('name');
    worksheets.load('items/name');
    await context.sync();

    const collections = [];
    for (const worksheet of worksheets.items) {
      const shapes = worksheet.shapes;
      shapes.load(
        'items/left,items/top,items/width,items/height,' +
        'items/rotation,items/visible'
      );
      collections.push({ worksheet, shapes });
    }

    const started = performance.now();
    await context.sync();

    const result = {};

    for (const entry of collections) {
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

        const right =
          Number(shape.left) +
          width / 2.0 +
          rotatedWidth / 2.0;

        const bottom =
          Number(shape.top) +
          height / 2.0 +
          rotatedHeight / 2.0;

        maxRight = Math.max(maxRight, right);
        maxBottom = Math.max(maxBottom, bottom);
      }

      result[entry.worksheet.name] = {
        right: fixed(maxRight),
        bottom: fixed(maxBottom)
      };
    }

    return {
      workbook: workbook.name,
      sheets: result,
      geometry_total_ms: fixed(performance.now() - started),
      excel_api_requirement_set: '1.11'
    };
  });
}

async function postResult(payload) {
  const response = await fetch('/result', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload)
  });

  if (!response.ok) {
    throw new Error(
      `Result upload failed: HTTP ${response.status}`
    );
  }
}

async function closeBenchmarkWorkbook() {
  try {
    await Excel.run(async (context) => {
      context.workbook.close(Excel.CloseBehavior.skipSave);
    });
  } catch (_) {
    // Closing the workbook tears down the task-pane runtime. A late rejection
    // from that teardown is not a benchmark failure.
  }
}

postClientEvent('script_loaded');

Office.onReady(async (info) => {
  await postClientEvent('office_ready', { host: info.host });

  try {
    if (info.host !== Office.HostType.Excel) {
      throw new Error('This experiment requires Excel.');
    }

    if (!Office.context.requirements.isSetSupported('ExcelApi', '1.11')) {
      throw new Error('ExcelApi 1.11 is required for workbook-close batch automation.');
    }

    setStatus('Measuring drawing bounds...');
    await postClientEvent('measurement_started');
    const payload = await measureWorkbook();
    await postClientEvent('measurement_completed', {
      workbook: payload.workbook,
      geometry_total_ms: payload.geometry_total_ms
    });
    await postResult(payload);
    await postClientEvent('result_submitted', { workbook: payload.workbook });
    setStatus('Result submitted. Closing benchmark workbook...');
    await new Promise((resolve) => window.setTimeout(resolve, 100));
    await closeBenchmarkWorkbook();
  } catch (error) {
    const detail =
      error && error.stack
        ? error.stack
        : String(error);

    setStatus(`ERROR: ${detail}`);
    await postClientEvent('error', { message: detail });

    try {
      await fetch('/error', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ message: detail })
      });
    } catch (_) {
    }
  }
});
