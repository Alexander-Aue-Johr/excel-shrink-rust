/* global Office */
'use strict';

(function () {
  const state = {
    startedAt: performance.now(),
    completedImages: 0,
    totalImages: 0,
    totalCaptureMs: 0,
    completedBatches: 0,
    totalBatches: 0,
    batches: []
  };

  // Source Context patch: optional manual start gate for debugger setup.
  let modeLoaded = false;
  let diagnosticRunId = null;

  function byId(id) { return document.getElementById(id); }
  function fixed(value, digits = 1) {
    const number = Number(value);
    return Number.isFinite(number) ? number.toFixed(digits) : '—';
  }
  function log(message) {
    const now = new Date().toLocaleTimeString();
    const target = byId('technical-log');
    if (target) {
      target.textContent += `${now} ${message}\n`;
      target.scrollTop = target.scrollHeight;
    }
  }
  function setText(id, value) {
    const element = byId(id);
    if (element) element.textContent = value;
  }
  function setPhase(value) { setText('phase', value); log(value); }
  function setWorkbook(value) { setText('workbook-name', value || '—'); }
  function setConnection(ok, text) {
    const light = byId('server-light');
    if (light) light.className = `status-light ${ok ? 'good' : 'bad'}`;
    setText('server-status', text);
  }
  function updateProgress(completed, total) {
    state.completedImages = completed;
    state.totalImages = total;
    const pct = total > 0 ? Math.max(0, Math.min(100, completed * 100 / total)) : 0;
    const bar = byId('progress-bar');
    if (bar) bar.style.width = `${pct}%`;
    setText('progress-text', `${completed} / ${total} sheets`);

    const elapsedMs = performance.now() - state.startedAt;
    setText('elapsed', `${fixed(elapsedMs / 1000)} s`);
    if (completed > 0 && total > completed) {
      const per = elapsedMs / completed;
      setText('eta', `ETA ~${fixed((total - completed) * per / 1000)} s`);
    } else if (completed >= total && total > 0) {
      setText('eta', 'Done');
    }
  }
  function startBatch(index, total, sheets) {
    state.totalBatches = total;
    setText('batch-number', `${index} / ${total}`);
    setText('current-sheets', sheets.join('\n'));
    setPhase(`Rendering batch ${index} / ${total}`);
    renderBatchRows(index, total, 'running', null);
  }
  function completeBatch(index, total, count, renderMs, persistMs) {
    const totalMs = Number(renderMs) + Number(persistMs);
    state.completedBatches += 1;
    state.totalCaptureMs += totalMs;
    state.completedImages += count;
    setText('batch-summary', `${state.completedBatches} complete`);

    const average = state.completedImages > 0
      ? state.totalCaptureMs / state.completedImages
      : 0;
    setText('average-ms', fixed(average));
    updateProgress(state.completedImages, state.totalImages);
    renderBatchRows(index, total, 'done', {
      count, renderMs, persistMs, totalMs,
      perImage: count > 0 ? totalMs / count : 0
    });
  }
  function renderBatchRows(index, total, status, timing) {
    const list = byId('batch-list');
    if (!list) return;
    let row = byId(`batch-${index}`);
    if (!row) {
      row = document.createElement('div');
      row.id = `batch-${index}`;
      row.className = 'batch-row';
      row.innerHTML =
        `<span class="batch-icon"></span>` +
        `<span class="batch-detail"></span>` +
        `<span class="batch-time"></span>`;
      list.appendChild(row);
    }
    const icon = row.querySelector('.batch-icon');
    const detail = row.querySelector('.batch-detail');
    const time = row.querySelector('.batch-time');
    if (status === 'done') {
      icon.textContent = '✓';
      detail.textContent = `Batch ${index} · ${timing.count} images`;
      time.textContent = `${fixed(timing.totalMs)} ms · ${fixed(timing.perImage)} ms/img`;
    } else {
      icon.textContent = '●';
      detail.textContent = `Batch ${index}`;
      time.textContent = 'rendering…';
    }
  }
  function showAnalysis(complete, total, text) {
    const panel = byId('analysis-panel');
    if (panel) panel.classList.remove('hidden');
    setText('analysis-summary', `${complete} / ${total}`);
    setText('analysis-text', text);
  }

  window.ExperimentUi = {
    state,
    log,
    setPhase,
    setWorkbook,
    setConnection,
    updateProgress,
    startBatch,
    completeBatch,
    showAnalysis,
    setTotalImages(value) {
      state.totalImages = value;
      updateProgress(state.completedImages, value);
    },
    configure(config) {
      setText(
        'experiment-title',
        config.experiment === 'range-image'
          ? `Get Picture · Strategy ${config.strategy}`
          : config.experiment === 'range-image-height-limit'
            ? 'Get Picture · Height plateau'
            : config.experiment === 'range-image-calibration'
              ? 'Get Picture · Units and markers'
              : 'Get shape bounds'
      );
      setText('host-kind', config.host_kind || '—');
      setText('batch-size', config.batch_size || '—');
    },
    fail(message) {
      setConnection(false, 'Experiment host error');
      setPhase(`ERROR: ${message}`);
    }
  };

  async function postEvent(stage, detail = null) {
    try {
      await fetch('/client-event', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...(diagnosticRunId ? { 'X-VV-Run-Id': diagnosticRunId } : {}) },
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

  window.ExperimentEvents = { post: postEvent };

  // Source Context patch: Office.js diagnostic setup.
  async function configureOfficeDiagnostics(config) {
    if (config.extended_error_logging) {
      if (
        typeof window.OfficeExtension === 'undefined' ||
        !window.OfficeExtension.config
      ) {
        throw new Error(
          'Extended Office.js error logging was requested, but OfficeExtension.config is unavailable.'
        );
      }

      window.OfficeExtension.config.extendedErrorLogging = true;
      log('OfficeExtension.config.extendedErrorLogging = true');
      await postEvent('office_extended_error_logging_enabled', { enabled: true });
    }

    if (config.office_js_debug) {
      log('Using Microsoft office.debug.js for this task pane.');
      await postEvent('office_debug_library_enabled', { enabled: true });
    }
  }

  async function loadExperimentMode(config) {
    if (modeLoaded) return;
    modeLoaded = true;
    const load = (url) => new Promise((resolve, reject) => {
      const script = document.createElement('script');
      script.src = url;
      script.onload = resolve;
      script.onerror = () => reject(new Error('Could not load ' + url));
      document.head.appendChild(script);
    });
    const run = encodeURIComponent(config.diagnostic_run_id || String(Date.now()));
    if (config.diagnostic_run_id) await load('/render-diagnostics.js?run=' + run);
    if (config.experiment === 'range-image') {
      await load('/capture-geometry.js?run=' + run);
      await load('/safe-capture.js?run=' + run);
      await load('/context-validation.js?run=' + run);
      await load('/capture-policy.js?run=' + run);
      await load('/selective-overflow.js?run=' + run);
    }
    await load('/mode.js?run=' + run);
  }

  async function waitForManualStart(config) {
    const panel = byId('manual-start-panel');
    const button = byId('start-experiment');
    if (!panel || !button) {
      throw new Error('Manual-start UI is missing from the task pane.');
    }

    panel.classList.remove('hidden');
    window.ExperimentUi.setPhase(
      'Ready to debug · open DevTools, configure breakpoints, then click Start experiment'
    );
    await postEvent('manual_start_waiting', {
      experiment: config.experiment,
      strategy: config.strategy
    });

    await new Promise((resolve) => {
      button.addEventListener('click', () => {
        button.disabled = true;
        button.textContent = 'Starting…';
        resolve();
      }, { once: true });
    });

    panel.classList.add('hidden');
    window.ExperimentUi.setPhase('Starting experiment');
    await postEvent('manual_start_clicked', {
      experiment: config.experiment,
      strategy: config.strategy
    });
  }

  async function main() {
    const configResponse = await fetch('/config');
    if (!configResponse.ok) {
      throw new Error(`Config request failed: HTTP ${configResponse.status}`);
    }
    const config = await configResponse.json();
    diagnosticRunId = config.capture_run_id || config.diagnostic_run_id || null;

    await configureOfficeDiagnostics(config);
    window.ExperimentUi.configure(config);
    window.ExperimentUi.setConnection(true, 'Experiment host connected');
    await postEvent('shell_loaded', { experiment: config.experiment });

    if (config.wait_for_start) {
      await waitForManualStart(config);
    }

    await loadExperimentMode(config);
  }

  window.setInterval(() => {
    postEvent('heartbeat', {
      phase: byId('phase') ? byId('phase').textContent : null,
      elapsed_ms: performance.now() - state.startedAt,
      completed_images: state.completedImages,
      total_images: state.totalImages
    });
  }, 5000);

  async function startWhenOfficeIsAvailable() {
    const started = performance.now();
    while (!window.Office || typeof window.Office.onReady !== 'function') {
      if (performance.now() - started > 30000) {
        const message = 'Office.js was unavailable for 30 seconds; no capture was started.';
        window.ExperimentUi.fail(message);
        await postEvent('office_bootstrap_failed', { message, office_defined: !!window.Office,
          scripts: Array.from(document.scripts).map(s => s.src).filter(Boolean) });
        return;
      }
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    let ready = false;
    let expired = false;
    const timer = setTimeout(() => {
      if (!ready) {
        expired = true;
        const message = 'Office.onReady did not complete within 60 seconds.';
        window.ExperimentUi.fail(message);
        postEvent('office_ready_timeout', {message});
      }
    }, 60000);
    window.Office.onReady(async (info) => {
    if (expired || ready) return;
    ready = true;
    clearTimeout(timer);
    await postEvent('office_ready', { host: info.host });
    try {
      if (info.host !== Office.HostType.Excel) {
        throw new Error('This experiment requires Excel.');
      }
      await main();
    } catch (error) {
      const detail = error && error.stack ? error.stack : String(error);
      window.ExperimentUi.fail(detail);
      await postEvent('error', { message: detail });
      try {
        await fetch('/error', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json', ...(diagnosticRunId ? { 'X-VV-Run-Id': diagnosticRunId } : {}) },
          body: JSON.stringify({ message: detail })
        });
      } catch (_) {
      }
    }
    });
  }
  startWhenOfficeIsAvailable().catch(error => {
    window.ExperimentUi.fail(String(error));
    postEvent('office_bootstrap_failed', { message: String(error) });
  });
}());
