/* Live package admission controls for the standalone native comparison host. */
export function captureControlRequest(form, runId) {
  if (!runId) throw new Error('No active comparison is available.');
  const request = { run_id: runId };
  for (const [key, minimum, maximum] of [
    ['geometry_batch_ranges', 1, 4096],
    ['capture_batch_ranges', 1, 64],
    ['capture_batch_megapixels', 32, 256],
  ]) {
    const value = Number(form.elements.namedItem(key).value);
    if (!Number.isInteger(value) || value < minimum || value > maximum) {
      throw new Error(`${key} must be an integer from ${minimum} to ${maximum}.`);
    }
    request[key] = value;
  }
  return request;
}

export function installCaptureControls({ api }) {
  const form = document.getElementById('live-capture-controls');
  const fields = document.getElementById('live-capture-fields');
  const status = document.getElementById('live-capture-status');
  let runId = null;
  let busy = false;

  async function refresh() {
    if (busy || !runId) return;
    const expected = runId;
    fields.disabled = true;
    try {
      const settings = await api('/api/capture/control');
      if (runId !== expected) return;
      if (!settings.available || settings.run_id !== expected) {
        throw new Error('The comparison changed. Waiting for its next status update.');
      }
      for (const key of ['geometry_batch_ranges', 'capture_batch_ranges', 'capture_batch_megapixels']) {
        form.elements.namedItem(key).value = settings[key];
      }
      fields.disabled = false;
      status.textContent = 'Changes apply to the active report at the next package boundary.';
    } catch (error) {
      if (runId === expected) status.textContent = error.message;
    }
  }

  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (busy) return;
    const expected = runId;
    try {
      const request = captureControlRequest(form, runId);
      busy = true;
      fields.disabled = true;
      await api('/api/capture/control', request);
      if (runId === expected) {
        status.textContent = 'Accepted. The active report will use these settings at the next package boundary.';
      }
    } catch (error) {
      if (runId === expected) status.textContent = error.message;
    } finally {
      busy = false;
      if (runId && runId !== expected) void refresh();
      else fields.disabled = !runId;
    }
  });
  document.getElementById('reload-capture-controls').addEventListener('click', refresh);
  return {
    render(state) {
      const next = state.active && state.telemetry?.status === 'running' ? state.telemetry.run_id : null;
      form.hidden = !next;
      if (next === runId) return;
      runId = next;
      fields.disabled = true;
      if (runId) void refresh();
    },
  };
}
