import { createProcessIcon } from './process-icon-shader.js';
import { createSignalMonitor } from './signal-monitor.js';
import { acceptComparison, captureProgress, comparisonFinished, heartbeatRow } from './targeted-model.js';

const node = (tag, className, text) => {
  const element = document.createElement(tag);
  element.className = className;
  if (text != null) element.textContent = text;
  return element;
};
const labels = {
  not_started: 'Not started', opening: 'Opening workbook', open: 'Workbook open',
  closing: 'Closing workbook', closed: 'Excel closed', failed: 'Exit unconfirmed', unavailable: 'Not available',
};

/** Create one lane card and retain its canvases throughout all telemetry updates. */
function createLaneCard() {
  const root = node('article', 'targeted-lane');
  const heading = node('div', 'targeted-lane-heading');
  const excel = createProcessIcon('excel');
  const xml = createProcessIcon('xml');
  excel.canvas.title = 'Excel: pulse while opening, steady while open, afterglow after confirmed exit.';
  xml.canvas.title = 'XML: host analysis of this workbook, with timestamp-based afterglow after completion.';
  const name = node('strong', '');
  heading.append(excel.canvas, name, xml.canvas);
  const state = node('p', 'targeted-process-state');
  const current = node('p', 'targeted-current');
  const progress = node('progress', 'targeted-progress');
  const progressText = node('p', 'targeted-progress-label');
  const signals = createSignalMonitor({ compact: true, heartbeatOnly: true });
  const source = node('p', 'targeted-source', 'Process heartbeat: exact PID alive. This does not prove Excel is responsive or making progress.');
  root.append(heading, state, current, progress, progressText, signals.element, source);
  return {
    element: root,
    update(comparison, lane, connected) {
      name.textContent = lane.label;
      state.textContent = [labels[lane.lifecycle] || lane.lifecycle, lane.pid ? `PID ${lane.pid}` : null].filter(Boolean).join(' · ');
      current.textContent = [lane.current_sheet, lane.current_range].filter(Boolean).join(' · ');
      current.hidden = !current.textContent;
      const work = captureProgress(lane);
      if (work.known) { progress.max = Math.max(1, work.total); progress.value = work.done; }
      else progress.removeAttribute('value');
      progressText.textContent = lane.lifecycle === 'unavailable' ? 'Check Performance was not run.' : work.text;
      progress.hidden = lane.lifecycle === 'unavailable';
      signals.element.hidden = lane.lifecycle === 'unavailable';
      source.hidden = lane.lifecycle === 'unavailable';
      signals.update(heartbeatRow(comparison, lane), { connected, live: !comparisonFinished(comparison) });
      excel.update(lane);
      xml.update(lane);
    },
    destroy() { excel.destroy(); xml.destroy(); signals.destroy(); root.remove(); },
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
  return {
    element: root,
    update(comparison) {
      const columns = comparison.columns || [];
      const next = JSON.stringify([columns, comparison.lanes.map(lane => [lane.id, lane.label])]);
      if (signature !== next) {
        signature = next;
        cells.clear();
        const heading = node('tr', '');
        heading.append(node('th', '', 'Workbook'));
        for (const column of columns) {
          const item = node('th', 'targeted-arrow', column.label);
          item.scope = 'col'; item.title = column.description;
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
      for (const lane of comparison.lanes) for (const column of columns) {
        const item = cells.get(`${lane.id}/${column.id}`);
        const status = lane.steps?.[column.id] || (lane.lifecycle === 'unavailable' ? 'unavailable' : 'pending');
        item.cell.dataset.step = status;
        item.cell.title = `${lane.label} · ${status}: ${item.description}`;
        item.check.checked = status === 'done';
        item.check.indeterminate = status === 'running' || status === 'skipped' || status === 'unavailable';
        item.check.setAttribute('aria-checked', status === 'done' ? 'true' : status === 'running' ? 'mixed' : 'false');
      }
    },
  };
}

/** A workbook group owns its lane cards until bounded history eviction. */
function createComparisonCard() {
  const root = node('article', 'targeted-workbook');
  const heading = node('header', 'targeted-workbook-heading');
  const name = node('h3', '');
  const outcome = node('strong', 'targeted-outcome');
  heading.append(name, outcome);
  const explanation = node('p', 'targeted-explanation');
  const error = node('p', 'targeted-error');
  const lanes = node('div', 'targeted-lanes');
  const flow = createFlowDiagram();
  const controls = node('p', 'targeted-controls-explanation',
    'Before: Original ↔ Default. After Default rebuild: Original-before ↔ Default-after. '
    + 'Drift control: Original-before ↔ Original-after. Pre-rebuild states stay open. Rebuilds use independent copies; Original is rebuilt last. Opening/saving and optimization XML controls are separate.');
  const legend = node('p', 'targeted-flow-legend', '☑ Done · ◩ Running · □ Pending · muted − Not performed / unavailable. Hover a column or checkbox for its purpose.');
  root.append(heading, explanation, error, lanes, flow.element, legend, controls);
  const laneViews = new Map();
  return {
    element: root,
    update(comparison, connected) {
      name.textContent = comparison.workbook || 'Targeted workbook comparison';
      const finished = comparisonFinished(comparison);
      root.dataset.finished = String(finished);
      outcome.textContent = comparison.status.replaceAll('_', ' ');
      const count = comparison.candidate_count;
      const selection = count == null ? 'Discovering candidate regions'
        : `${count} candidate regions · shared candidate regions per retained state`;
      const performanceEnabled = comparison.lanes.some(lane => lane.id === 'CP' && lane.lifecycle !== 'unavailable');
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
      const processLanes = comparison.lanes.filter(lane => lane.lifecycle !== 'unavailable');
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
        outcome.textContent += ` · ${comparison.report.before_differences} before / ${comparison.report.after_differences} after differences · ${comparison.report.original_recalc_drift} original drift`;
        if (comparison.report.check_performance_differences != null) {
          outcome.textContent += ` · ${comparison.report.check_performance_differences} Check Performance differences`;
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
    for (const item of all) {
      if (!displayed.has(item.run_id) && !views.has(item.run_id)) continue;
      if (!views.has(item.run_id)) views.set(item.run_id, createComparisonCard());
      const view = views.get(item.run_id);
      view.element.hidden = !displayed.has(item.run_id);
      view.update(item, connected);
    }
    for (const [parent, items] of [[active, running], [completed, ended]]) {
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
