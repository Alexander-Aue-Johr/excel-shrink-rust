import { createProcessIcon } from './process-icon-shader.js';
import { createSignalMonitor } from './signal-monitor.js';
import { createClipboardButton } from './inspection-clipboard.js';
import { acceptComparison, captureProgress, comparisonFinished, heartbeatRow, phaseComplete, processCards } from './targeted-model.js';

const node = (tag, className, text) => {
  const element = document.createElement(tag);
  element.className = className;
  if (text != null) element.textContent = text;
  return element;
};
const labels = {
  not_started: 'Not started', opening: 'Opening workbook', open: 'Workbook open',
  closing: 'Closing workbook', closed: 'Excel closed', failed: 'Exit unconfirmed', unavailable: 'Not available',
  skipped: 'Not required',
};

/** Create one lane card and retain its canvases throughout all telemetry updates. */
function createLaneCard() {
  const root = node('article', 'targeted-lane');
  const heading = node('div', 'targeted-lane-heading');
  const excel = createProcessIcon('excel');
  const xml = createProcessIcon('xml');
  const process = createProcessIcon('process');
  process.canvas.title = 'Windows process: exact PID and creation time confirmed alive. This does not prove Excel responds.';
  excel.canvas.title = 'Excel: pulse while opening, steady while open, afterglow after confirmed exit.';
  xml.canvas.title = 'XML: host analysis of this workbook, with timestamp-based afterglow after completion.';
  const name = node('strong', '');
  heading.append(excel.canvas, name, process.canvas, xml.canvas);
  const state = node('p', 'targeted-process-state');
  const current = node('p', 'targeted-current');
  const progress = node('progress', 'targeted-progress');
  const progressText = node('p', 'targeted-progress-label');
  const signals = createSignalMonitor({ compact: true, heartbeatOnly: true });
  const source = node('p', 'targeted-source');
  root.append(heading, state, current, progress, progressText, signals.element, source);
  return {
    element: root,
    update(comparison, lane, connected) {
      name.textContent = lane.label;
      state.textContent = [labels[lane.lifecycle] || lane.lifecycle, lane.pid ? `PID ${lane.pid}` : null].filter(Boolean).join(' · ');
      current.textContent = [lane.decision_note, lane.current_sheet, lane.current_range].filter(Boolean).join(' · ');
      current.hidden = !current.textContent;
      const work = captureProgress(lane);
      if (work.known) { progress.max = Math.max(1, work.total); progress.value = work.done; }
      else progress.removeAttribute('value');
      progressText.textContent = lane.lifecycle === 'unavailable' ? 'Check Performance was not run.' : work.text;
      progress.hidden = ['unavailable', 'skipped'].includes(lane.lifecycle);
      progressText.hidden = lane.lifecycle === 'skipped';
      signals.element.hidden = ['unavailable', 'skipped'].includes(lane.lifecycle);
      source.hidden = ['unavailable', 'skipped'].includes(lane.lifecycle);
      source.textContent = Number.isFinite(lane.addin_heartbeat_at)
        ? `Last Office.js heartbeat: ${Math.max(0, Math.floor((Date.now() - lane.addin_heartbeat_at) / 1000))} s ago. Heartbeats do not prove calculation progress.`
        : lane.lifecycle === 'opening' ? 'Waiting for first Office.js heartbeat while opening.'
          : 'Office.js heartbeat data unavailable for this card. The cyan chip indicates process liveness separately.';
      signals.update(heartbeatRow(comparison, lane), { connected, live: !comparisonFinished(comparison) });
      excel.update(lane);
      xml.update(lane);
      process.update(lane);
    },
    destroy() { excel.destroy(); xml.destroy(); process.destroy(); signals.destroy(); root.remove(); },
  };
}

/** Update existing checkboxes; arrow columns explain the control experiment. */
function createFlowDiagram() {
  const root = node('div', 'targeted-flow');
  root.tabIndex = 0;
  root.setAttribute('role', 'region');
  root.setAttribute('aria-label', 'Comparison workflow; scroll horizontally for all phases');
  const table = node('table', '');
  const head = node('thead', '');
  const body = node('tbody', '');
  table.append(head, body);
  root.append(table);
  let signature = '';
  const cells = new Map();
  const headers = new Map();
  return {
    element: root,
    update(comparison) {
      const columns = comparison.columns || [];
      const next = JSON.stringify([columns, comparison.lanes.map(lane => [lane.id, lane.label])]);
      if (signature !== next) {
        signature = next;
        cells.clear();
        headers.clear();
        const heading = node('tr', '');
        heading.append(node('th', '', 'Workbook'));
        for (const column of columns) {
          const item = node('th', 'targeted-arrow', column.label);
          item.scope = 'col'; item.title = column.description;
          headers.set(column.id, item);
          heading.append(item);
        }
        head.replaceChildren(heading);
        body.replaceChildren();
        for (const lane of comparison.lanes) {
          const row = node('tr', '');
          const label = node('th', '', lane.label);
          label.scope = 'row'; row.append(label);
          for (const column of columns) {
            const cell = node('td', '');
            const check = node('input', '');
            check.type = 'checkbox'; check.disabled = true;
            check.setAttribute('aria-label', `${lane.label}: ${column.label}`);
            cell.append(check); row.append(cell);
            cells.set(`${lane.id}/${column.id}`, { cell, check, description: column.description });
          }
          body.append(row);
        }
      }
      for (const column of columns) {
        headers.get(column.id).dataset.complete = String(phaseComplete(comparison.lanes, column.id));
      }
      for (const lane of comparison.lanes) for (const column of columns) {
        const item = cells.get(`${lane.id}/${column.id}`);
        const status = lane.steps?.[column.id] || (lane.lifecycle === 'unavailable' ? 'unavailable' : 'pending');
        item.cell.dataset.step = status;
        item.cell.title = `${lane.label} · ${status}: ${(status === 'skipped' && lane.decision_note) || item.description}`;
        item.check.checked = status === 'done';
        item.check.indeterminate = status === 'running' || status === 'skipped' || status === 'unavailable';
        item.check.setAttribute('aria-checked', status === 'done' ? 'true' : status === 'running' ? 'mixed' : 'false');
      }
    },
  };
}

/** A visible workbook group owns its lane cards; hidden history keeps only data. */
function createComparisonCard() {
  const root = node('article', 'targeted-workbook');
  const heading = node('header', 'targeted-workbook-heading');
  const name = node('h3', '');
  const outcome = node('strong', 'targeted-outcome');
  heading.append(name, outcome);
  const explanation = node('p', 'targeted-explanation');
  const error = node('p', 'targeted-error');
  const copyNotice = node('p', 'targeted-copy-status');
  copyNotice.setAttribute('role', 'status');
  let groupId = null;
  const copy = createClipboardButton(() => groupId, copyNotice);
  heading.append(copy);
  const lanes = node('div', 'targeted-lanes');
  const flow = createFlowDiagram();
  const controls = node('p', 'targeted-controls-explanation',
    'Rebuild Original and Default in their existing Excel processes, then compare selected regions and all charts. '
    + 'Optional Check Performance is rebuilt and compared against rebuilt Original. Pre-rebuild XML controls remain available for follow-up.');
  const legend = node('p', 'targeted-flow-legend', '☑ Done · ◩ Running · □ Pending · muted − Not performed / unavailable. Hover a column or checkbox for its purpose.');
  root.append(heading, explanation, error, copyNotice, lanes, flow.element, legend, controls);
  const laneViews = new Map();
  return {
    element: root,
    update(comparison, connected) {
      const parts = String(comparison.report_directory || '').split(/[\\/]+/);
      const evidence = parts.lastIndexOf('evidence');
      groupId = evidence >= 0 && /^[a-f0-9]{20}$/.test(parts[evidence + 1]) ? parts[evidence + 1] : null;
      copy.hidden = !groupId;
      name.textContent = comparison.workbook || 'Targeted workbook comparison';
      const finished = comparisonFinished(comparison);
      root.dataset.finished = String(finished);
      outcome.textContent = comparison.status.replaceAll('_', ' ');
      const count = comparison.candidate_count;
      const selection = count == null ? 'Discovering candidate regions'
        : `${count} candidate regions · shared candidate regions per retained state`;
      const performanceEnabled = comparison.lanes.some(lane => lane.id === 'CP' && !['unavailable', 'skipped'].includes(lane.lifecycle));
      const performanceStatus = comparison.report?.check_performance || (performanceEnabled ? 'enabled' : 'not run');
      explanation.textContent = `${selection}. Limited candidate coverage. Check Performance: ${performanceStatus.replaceAll('_', ' ')}.`;
      if (comparison.report?.chart_comparisons != null) {
        explanation.textContent += ` Charts: ${comparison.report.chart_comparisons} comparisons · ${comparison.report.chart_differences} differences.`;
      }
      if (comparison.report?.cf_excluded_decisions != null) {
        explanation.textContent += ` CF decisions across state pairs: ${comparison.report.cf_excluded_decisions} excluded / ${comparison.report.cf_partial_decisions} partial / ${comparison.report.cf_capture_decisions} capture.`;
      }
      error.textContent = comparison.error || '';
      error.hidden = !comparison.error;
      root.title = (comparison.report?.limitations || []).join('\n');
      const processLanes = processCards(comparison.lanes);
      const ids = new Set(processLanes.map(lane => lane.id));
      for (const [id, view] of laneViews) if (!ids.has(id)) { view.destroy(); laneViews.delete(id); }
      for (const lane of processLanes) {
        if (!laneViews.has(lane.id)) {
          const view = createLaneCard(); laneViews.set(lane.id, view); lanes.append(view.element);
        }
        laneViews.get(lane.id).update(comparison, lane, connected);
      }
      flow.update(comparison);
      if (finished && comparison.report?.before_differences != null) {
        const report = comparison.report;
        outcome.textContent += report.input_identity?.status === 'identical'
          ? ' · Default unchanged (identical input bytes)'
          : report.before_differences == null ? ' · Pre-rebuild images omitted' : ` · Default: ${report.before_differences} differences`;
        outcome.textContent += report.after_differences == null
          ? ' · Default rebuild not performed'
          : ` · Rebuilt Default: ${report.after_differences} differences`;
        outcome.textContent += report.original_recalc_drift == null
          ? ' · Original rebuild not performed'
          : ` · Original rebuild drift: ${report.original_recalc_drift}`;
        if (comparison.report.check_performance_differences != null) {
          outcome.textContent += ` · ${comparison.report.check_performance_differences} Check Performance differences`;
        }
        if (comparison.report.check_performance_cache?.status === 'hit') {
          outcome.textContent += ' · Check Performance from cache (historical generation measurements)';
        } else if (comparison.report.check_performance_cache?.status === 'generated') {
          outcome.textContent += ' · Check Performance artifacts cached';
        }
        if (comparison.report.check_performance_cache?.warning) {
          outcome.textContent += ' · Cache warning (see report)';
        }
      }
    },
    destroy() { for (const view of laneViews.values()) view.destroy(); root.remove(); },
  };
}

/** Retain active and recent completed comparisons across workbook selection. */
export function createTargetedPanel(root) {
  root.className = 'targeted-comparison';
  const heading = node('header', 'targeted-panel-heading');
  const title = node('h2', '', 'Targeted comparisons');
  const summary = node('span', '');
  const fullscreen = node('button', '', 'Full screen');
  fullscreen.type = 'button';
  fullscreen.disabled = !root.requestFullscreen;
  heading.append(title, summary, fullscreen);
  const notice = node('p', 'targeted-error');
  notice.hidden = true; notice.setAttribute('role', 'status');
  const activeTitle = node('h3', '', 'Active workbooks');
  const active = node('div', 'targeted-active');
  const completedTitle = node('h3', '', 'Completed workbooks');
  const completed = node('div', 'targeted-completed');
  const more = node('button', 'targeted-more', '…');
  more.type = 'button';
  root.replaceChildren(heading, notice, activeTitle, active, completedTitle, completed, more);
  const records = new Map();
  const views = new Map();
  const retired = new Set();
  let completedLimit = 3;
  let connected = true;
  let nextRefresh = 0;
  let refreshing = false;
  let supported = false;

  function render() {
    root.hidden = records.size === 0;
    const all = [...records.values()];
    const running = all.filter(item => !comparisonFinished(item)).sort((a, b) => a.started_at - b.started_at);
    const ended = all.filter(comparisonFinished).sort((a, b) => b.finished_at - a.finished_at);
    const visible = document.fullscreenElement === root ? ended : ended.slice(0, completedLimit);
    const displayed = new Set([...running, ...visible].map(item => item.run_id));
    const before = new Map([...views].map(([id, view]) => [id, view.element.getBoundingClientRect()]));
    // Hidden history retains report records, not canvas and observer trees.
    for (const [id, view] of views) {
      if (displayed.has(id)) continue;
      view.destroy();
      views.delete(id);
    }
    for (const item of [...running, ...visible]) {
      if (!views.has(item.run_id)) views.set(item.run_id, createComparisonCard());
      views.get(item.run_id).update(item, connected);
    }
    for (const [parent, items] of [[active, running], [completed, visible]]) {
      items.filter(item => views.has(item.run_id)).forEach((item, index) => {
        const element = views.get(item.run_id).element;
        if (parent.children[index] !== element) parent.insertBefore(element, parent.children[index] || null);
        const previous = before.get(item.run_id);
        if (previous?.width && !element.hidden && !matchMedia('(prefers-reduced-motion: reduce)').matches) {
          const after = element.getBoundingClientRect();
          const x = previous.left - after.left, y = previous.top - after.top;
          if (x || y) element.animate([{ transform: `translate(${x}px, ${y}px)` }, { transform: 'none' }], { duration: 450, easing: 'ease-out' });
        }
      });
    }
    activeTitle.hidden = running.length === 0;
    completedTitle.hidden = ended.length === 0;
    summary.textContent = `${running.length} active · ${ended.length} completed · latest 100 retained`;
    more.hidden = visible.length >= ended.length;
    more.setAttribute('aria-label', `Show more completed comparisons (${ended.length - visible.length} hidden)`);
    more.title = more.getAttribute('aria-label');
  }

  function accept(comparison) {
    if (retired.has(comparison?.run_id)) return;
    const next = acceptComparison(records.get(comparison?.run_id), comparison);
    if (!next) return;
    records.set(next.run_id, next);
    // Only ended history is evicted; an active comparison never disappears due to a limit.
    const ended = [...records.values()].filter(comparisonFinished).sort((a, b) => b.finished_at - a.finished_at);
    for (const item of ended.slice(100)) {
      records.delete(item.run_id); views.get(item.run_id)?.destroy(); views.delete(item.run_id);
    }
  }

  more.addEventListener('click', () => { completedLimit += 6; render(); });
  fullscreen.addEventListener('click', async () => {
    try {
      if (document.fullscreenElement === root) await document.exitFullscreen();
      else await root.requestFullscreen();
    } catch (error) { notice.textContent = `Fullscreen failed: ${error.message}`; notice.hidden = false; }
  });
  document.addEventListener('fullscreenchange', () => {
    fullscreen.textContent = document.fullscreenElement === root ? 'Exit full screen' : 'Full screen';
    render();
  });
  return {
    get supported() { return supported; },
    update(comparison) { if (comparison?.run_id) { accept(comparison); render(); } },
    async refresh() {
      if (refreshing || document.hidden || Date.now() < nextRefresh) return;
      refreshing = true; nextRefresh = Date.now() + 3000;
      try {
        const response = await fetch('/api/targeted-comparisons', { cache: 'no-store' });
        if (response.status === 404) {
          supported = false;
          nextRefresh = Date.now() + 60000;
          return; // An older host still serves ordinary corpus work.
        }
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const snapshot = await response.json();
        supported = true;
        for (const id of snapshot.retired_run_ids || []) {
          retired.add(id);
          while (retired.size > 512) retired.delete(retired.values().next().value);
          records.delete(id);
          views.get(id)?.destroy();
          views.delete(id);
        }
        for (const comparison of snapshot.comparisons || []) accept(comparison);
        connected = true; notice.hidden = true;
        render();
      } catch (error) {
        connected = false;
        notice.textContent = `Comparison telemetry unavailable: ${error.message}. Keeping the last snapshot.`;
        notice.hidden = false; render();
      } finally { refreshing = false; }
    },
  };
}
