import { readDiagnosticResponse } from './diagnostic-response.js';

let opening = false;

/** One global inspection pair follows the selected workbook instead of accumulating windows. */
export function createRegionInspectButton(context, notice) {
  const button = document.createElement('button');
  button.type = 'button';
  button.textContent = 'Inspect pair in Excel';
  button.title = 'Open read-only prepared Original and Default copies side by side and select this range. Reuses the inspection pair.';
  button.addEventListener('click', async () => {
    const target = context();
    if (!target?.id || !target.sheet || !target.range) return;
    if (opening) { notice.textContent = 'Another inspection is still opening. Please wait.'; return; }
    opening = true;
    button.disabled = true;
    notice.textContent = `Opening inspection copies · ${target.sheet}!${target.range}…`;
    try {
      const response = await fetch(`/api/groups/${encodeURIComponent(target.id)}/inspect-region`, {
        method: 'POST', headers: {'Content-Type': 'application/json'},
        body: JSON.stringify({sheet: target.sheet, range: target.range}),
      });
      const result = await readDiagnosticResponse(response, 'Excel region inspection');
      if (result.ok !== true) throw new Error(result.error || 'Inspection was not confirmed.');
      notice.textContent = `${target.sheet}!${target.range} selected · Original PID ${result.original.pid}`
        + ` · Default PID ${result.default.pid}. Prepared read-only copies; captured rebuilt states may have different cached values.`;
    } catch (error) {
      notice.textContent = `Cannot inspect region: ${error.message}`;
    } finally {
      opening = false;
      button.disabled = false;
    }
  });
  return button;
}

/** Fetch a bounded page only on demand; live telemetry updates leave this list alone. */
export function createRegionList(groupId) {
  const root = document.createElement('details');
  root.className = 'capture-region-list';
  const summary = document.createElement('summary');
  summary.textContent = 'Capture regions';
  const controls = document.createElement('div');
  controls.className = 'capture-region-controls';
  const note = document.createElement('p');
  note.className = 'process-metrics-note';
  note.textContent = 'Inspect prepared copies without recapturing. Switching cards closes the previous inspection workbooks without saving.';
  const notice = document.createElement('p');
  notice.setAttribute('role', 'status');
  const list = document.createElement('div');
  list.className = 'capture-region-rows';
  const previous = document.createElement('button');
  const next = document.createElement('button');
  const refresh = document.createElement('button');
  previous.textContent = 'Previous 100';
  next.textContent = 'Next 100';
  refresh.textContent = 'Load / refresh regions';
  previous.disabled = next.disabled = true;
  controls.append(previous, refresh, next);
  root.append(summary, note, controls, notice, list);
  let offset = 0;
  let loaded = false;
  async function load(requestedOffset) {
    const id = groupId();
    if (!id) return;
    previous.disabled = next.disabled = refresh.disabled = true;
    try {
      const response = await fetch(`/api/groups/${encodeURIComponent(id)}/capture-regions?offset=${requestedOffset}`);
      const data = await readDiagnosticResponse(response, 'Capture region list');
      if (id !== groupId()) return;
      if (requestedOffset > 0 && requestedOffset >= data.total) {
        await load(0);
        return;
      }
      offset = data.offset;
      loaded = true;
      const rows = data.regions.map(region => {
        const row = document.createElement('div');
        row.className = 'capture-region-row';
        const label = document.createElement('span');
        label.textContent = `${region.sheet}!${region.range}`;
        row.append(label, createRegionInspectButton(() => ({id, ...region}), notice));
        return row;
      });
      list.replaceChildren(...rows);
      notice.textContent = `${offset + (rows.length ? 1 : 0)}–${offset + rows.length} of ${data.total}`
        + ` · ${data.stage.replaceAll('_', ' ')}`;
      previous.disabled = offset === 0;
      next.disabled = offset + rows.length >= data.total;
    } catch (error) {
      notice.textContent = `Regions unavailable: ${error.message}`;
    } finally {
      refresh.disabled = false;
    }
  }
  previous.addEventListener('click', () => load(Math.max(0, offset - 100)));
  next.addEventListener('click', () => load(offset + 100));
  refresh.addEventListener('click', () => load(offset));
  root.addEventListener('toggle', () => { if (root.open && !loaded) load(0); });
  return root;
}
