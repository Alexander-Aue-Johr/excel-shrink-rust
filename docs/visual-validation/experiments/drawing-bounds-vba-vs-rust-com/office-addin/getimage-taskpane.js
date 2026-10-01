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

async function getWorkbookName() {
  return Excel.run(async (context) => {
    const workbook = context.workbook;
    workbook.load('name');
    await context.sync();
    return workbook.name;
  });
}

async function getCapturePlan(workbookName) {
  const response = await fetch(
    `/capture-plan?workbook=${encodeURIComponent(workbookName)}`
  );

  if (!response.ok) {
    throw new Error(
      `Capture-plan request failed: HTTP ${response.status}`
    );
  }

  return response.json();
}

async function renderBatch(items) {
  return Excel.run(async (context) => {
    const workbook = context.workbook;
    const requests = [];
    const started = performance.now();

    for (const item of items) {
      const worksheet = workbook.worksheets.getItem(item.sheet);
      const range = worksheet.getRange(item.address);
      const image = range.getImage();

      requests.push({
        item,
        image
      });
    }

    await context.sync();
    const renderMs = performance.now() - started;

    return {
      render_ms: renderMs,
      images: requests.map((entry) => ({
        id: entry.item.id,
        base64: entry.image.value
      }))
    };
  });
}

async function persistBatch(workbookName, images) {
  const started = performance.now();

  const response = await fetch('/image-batch', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      workbook: workbookName,
      images
    })
  });

  if (!response.ok) {
    const body = await response.text();
    throw new Error(
      `Image persistence failed: HTTP ${response.status}: ${body}`
    );
  }

  const payload = await response.json();

  return {
    persist_ms: performance.now() - started,
    captures: payload.captures || []
  };
}

async function postResult(payload) {
  const response = await fetch('/result', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload)
  });

  if (!response.ok) {
    const body = await response.text();
    throw new Error(
      `Result upload failed: HTTP ${response.status}: ${body}`
    );
  }
}

// Workbook/process teardown is deliberately owned by the PowerShell host.
 // Closing from the task-pane runtime immediately after /result made the UI vanish
 // before the host had fully observed/settled the successful run and made repeated
 // Excel/WebView startup less deterministic.

async function runGetImageBenchmark() {
  const workbookName = await getWorkbookName();
  const plan = await getCapturePlan(workbookName);
  const captures = Array.isArray(plan.captures) ? plan.captures : [];
  const batchSize = Math.max(1, Number(plan.batch_size || 1));

  let renderTotalMs = 0;
  let persistTotalMs = 0;
  const persistedCaptures = [];

  for (let offset = 0; offset < captures.length; offset += batchSize) {
    const batch = captures.slice(offset, offset + batchSize);

    const rendered = await renderBatch(batch);
    renderTotalMs += rendered.render_ms;

    const persisted = await persistBatch(workbookName, rendered.images);
    persistTotalMs += persisted.persist_ms;
    persistedCaptures.push(...persisted.captures);
  }

  return {
    workbook: workbookName,
    render_total_ms: fixed(renderTotalMs),
    persist_total_ms: fixed(persistTotalMs),
    capture_total_ms: fixed(renderTotalMs + persistTotalMs),
    image_batch_size: batchSize,
    capture_count: persistedCaptures.length,
    captures: persistedCaptures,
    excel_api_requirement_set: '1.11'
  };
}

postClientEvent('script_loaded');

Office.onReady(async (info) => {
  await postClientEvent('office_ready', { host: info.host });

  try {
    if (info.host !== Office.HostType.Excel) {
      throw new Error('This experiment requires Excel.');
    }

    if (!Office.context.requirements.isSetSupported('ExcelApi', '1.11')) {
      throw new Error(
        'ExcelApi 1.11 is required for Range.getImage() plus unattended workbook closing.'
      );
    }

    setStatus('Rendering ranges with Range.getImage()...');
    await postClientEvent('measurement_started');

    const payload = await runGetImageBenchmark();

    await postClientEvent('measurement_completed', {
      workbook: payload.workbook,
      capture_total_ms: payload.capture_total_ms,
      render_total_ms: payload.render_total_ms,
      persist_total_ms: payload.persist_total_ms,
      capture_count: payload.capture_count,
      image_batch_size: payload.image_batch_size
    });

    await postResult(payload);

    await postClientEvent('result_submitted', {
      workbook: payload.workbook
    });

    setStatus('Result submitted. Waiting for benchmark host teardown...');
    await postClientEvent('waiting_for_host_teardown', {
      workbook: payload.workbook
    });
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
