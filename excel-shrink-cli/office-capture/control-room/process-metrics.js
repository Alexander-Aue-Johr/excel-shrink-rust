import { openingMetrics, processFamily, processTimings, relativeCost, seconds } from './process-metrics-model.js';

const node = (tag, className = '', text) => {
  const element = document.createElement(tag);
  element.className = className;
  if (text != null) element.textContent = text;
  return element;
};

function createMetric(label, property, format) {
  const root = node('section', 'process-metric');
  const heading = node('strong', '', label);
  const value = node('span', 'process-metric-value');
  const delta = node('span', 'process-metric-delta');
  const chart = node('div', 'process-metric-chart');
  const bars = ['Original', 'Default'].map(name => {
    const row = node('div', 'process-metric-bar-row');
    const track = node('div', 'process-metric-track');
    const bar = node('span', 'process-metric-bar');
    track.append(bar);
    row.append(node('span', '', name), track);
    chart.append(row);
    return { row, bar };
  });
  root.append(heading, value, chart, delta);
  return {
    element: root,
    update(comparison, lane) {
      const ownFamily = processFamily(lane.id)[0];
      const original = openingMetrics(comparison, 'O')?.[property];
      const variant = openingMetrics(comparison, ownFamily === 'CP' ? 'CP' : 'SD')?.[property];
      const values = [original, variant];
      const own = ownFamily === 'O' ? 0 : 1;
      const cost = relativeCost(values[own], values[1 - own]);
      const maximum = Math.max(...values.filter(Number.isFinite), 0);
      value.textContent = Number.isFinite(values[own]) ? format(values[own]) : 'Not recorded';
      delta.textContent = cost?.text || 'Comparison measurement unavailable';
      root.dataset.tone = cost?.tone || 'unknown';
      bars.forEach(({ row, bar }, index) => {
        row.dataset.primary = String(index === own);
        row.firstChild.textContent = index === 0 ? 'Original' : ownFamily === 'CP' ? 'Check Performance' : 'Default';
        bar.style.width = maximum > 0 && Number.isFinite(values[index]) ? `${100 * values[index] / maximum}%` : '0%';
        row.title = Number.isFinite(values[index]) ? format(values[index]) : 'Not recorded';
      });
    },
  };
}

/** Retain details nodes while live clocks update; opening a section never resets it. */
export function createProcessMetrics() {
  const root = node('div', 'process-metrics');
  const opening = createMetric('Opening', 'startup_seconds', value => `${value.toFixed(2)} s`);
  const memory = createMetric('Start RAM', 'startup_memory_working_set_bytes', value => `${(value / 1048576).toFixed(1)} MiB`);
  const note = node('p', 'process-metrics-note', 'Launch → add-in ready. Start RAM is the working set sampled at readiness.');
  const details = node('details', 'process-timings');
  const summary = node('summary', '', 'Timings');
  const explanation = node('p', 'process-metrics-note', 'Wall times may overlap. Upload includes PNG byte preparation. Missing timings were not recorded by that runner.');
  const rows = node('div', 'process-timing-rows');
  const entries = new Map();
  details.append(summary, explanation, rows);
  root.append(opening.element, memory.element, note, details);
  return {
    element: root,
    update(comparison, lane) {
      opening.update(comparison, lane);
      memory.update(comparison, lane);
      const timings = processTimings(comparison, lane.id);
      const labels = new Map((comparison.columns || []).map(column => [column.id, column.label]));
      const values = timings.phases.map(phase => ({
        key: `phase/${phase.id}`, label: labels.get(phase.id) || phase.id,
        text: `${seconds(phase.elapsed_ms)}${phase.running ? ' · running' : ''}`,
      }));
      for (const operation of timings.operations) {
        values.push({
          key: `operation/${operation.lane}/${operation.sheet}/${operation.operation}`,
          label: `${operation.sheet || 'Workbook'} · ${operation.operation.replaceAll('_', ' ')}`,
          text: `${seconds(operation.elapsed_ms)} · ${operation.calls} calls${operation.running ? ' · running' : ''}`
            + ` · geometry ${seconds(operation.geometry_ms)} · render ${seconds(operation.render_ms)}`
            + ` · upload ${seconds(operation.upload_ms)}`,
        });
      }
      const elapsed = kind => timings.operations.filter(item => item.operation === kind)
        .reduce((total, item) => total + item.elapsed_ms, 0);
      summary.textContent = timings.operations.length
        ? `Timings · geometry ${seconds(elapsed('measure_batch'))} · captures ${seconds(elapsed('capture_many'))}`
        : 'Timings · awaiting operation measurements';
      const keys = new Set(values.map(item => item.key));
      for (const [key, row] of entries) if (!keys.has(key)) { row.remove(); entries.delete(key); }
      for (const item of values) {
        let row = entries.get(item.key);
        if (!row) {
          row = node('p', 'process-timing-row');
          row.append(node('span'), node('span'));
          entries.set(item.key, row);
          rows.append(row);
        }
        row.children[0].textContent = item.label;
        row.children[1].textContent = item.text;
      }
    },
  };
}
