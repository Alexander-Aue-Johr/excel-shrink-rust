import { readDiagnosticResponse } from './diagnostic-response.js';
import { DiagnosticCache } from './diagnostic-cache.js';

const summaries = new DiagnosticCache({ maxEntries: 512, maxBytes: 256 * 1024 });
const retained = new DiagnosticCache({ maxEntries: 64, maxBytes: 8 * 1024 * 1024 });
const pending = new Map();
const MAX_PENDING_REQUESTS = 16;
const make = (tag, className, text) => {
  const element = document.createElement(tag);
  element.className = className;
  if (text != null) element.textContent = text;
  return element;
};

/** A dependency observation is not an assertion that all differences are volatile. */
export function formulaSummary(diagnosis) {
  if (!diagnosis || !diagnosis.volatile_paths) return '';
  return diagnosis.evidence === 'prepared_source_cached_values'
    ? 'Random/time dependency in source · captured values unavailable'
    : `${diagnosis.volatile_paths} random/time dependency paths · other differences remain unclassified`;
}

export function attachFormulaSummary(element, id) {
  element.dataset.formulaGroup = id;
  element.className = 'workbook-formula-summary';
  element.textContent = summaries.get(id) || '';
}

function remember(id, diagnosis) {
  if (!diagnosis.volatile_paths) return;
  summaries.set(id, formulaSummary(diagnosis));
  for (const element of document.querySelectorAll('.workbook-formula-summary')) {
    if (element.dataset.formulaGroup === id) element.textContent = formulaSummary(diagnosis);
  }
}

/** Split only the verified call span; all formula text stays text, never HTML. */
export function formulaSegments(formula, span) {
  if (!Array.isArray(span) || span.length !== 2 || !span.every(Number.isInteger)
      || span[0] < 0 || span[1] < span[0] || span[1] > formula.length) return [formula, '', ''];
  return [formula.slice(0, span[0]), formula.slice(span[0], span[1]), formula.slice(span[1])];
}

function formulaCode(node, pulsing) {
  const code = make('code', 'formula-expression');
  const [before, call, after] = formulaSegments(node.formula || '', node.volatile_span);
  code.append(document.createTextNode(before));
  if (call) code.append(make('span', pulsing ? 'formula-volatile-call' : 'formula-volatile-call pulse-paused', call));
  code.append(document.createTextNode(after));
  return code;
}

/** Cache resolved evidence, not promises that retain every visited graph forever. */
export function requestFormulaDiagnostics(id, region, reportId) {
  const key = `${id}/${reportId}/${region}`;
  const cached = retained.get(key);
  if (cached) return Promise.resolve(cached);
  if (pending.has(key)) return pending.get(key);
  if (pending.size >= MAX_PENDING_REQUESTS) {
    return Promise.reject(new Error('Formula inspection queue is full; retry this region.'));
  }
  const request = fetch(`/api/groups/${encodeURIComponent(id)}/formula-diagnostics?region=${encodeURIComponent(region)}`,
    { cache: 'no-store' }).then(async response => {
    const result = await readDiagnosticResponse(response, 'Formula diagnostics');
    if (result.report_id !== reportId) throw new Error('Comparison changed; reopen the atelier.');
    retained.set(key, result.diagnosis);
    return result.diagnosis;
  }).finally(() => pending.delete(key));
  pending.set(key, request);
  return request;
}

/** Render a separate read-only path, without marking or editing worksheet cells. */
export function createFormulaInspector(id, region, reportId) {
  const root = make('section', 'formula-inspector');
  root.append(make('h3', '', 'Formula dependencies'));
  const note = make('p', 'formula-evidence-note', 'Loading formula evidence…');
  root.append(note);
  requestFormulaDiagnostics(id, region.key, reportId).then(diagnosis => {
    remember(id, diagnosis);
    if (!root.isConnected) return;
    const source = diagnosis.evidence === 'prepared_source_cached_values';
    note.textContent = diagnosis.reason || diagnosis.note || 'No formula evidence retained.';
    root.classList.toggle('formula-source-only', source);
    if (diagnosis.changed_value_cells != null) {
      root.append(make('p', '', `${diagnosis.changed_value_cells} changed saved cell values · `
        + `${diagnosis.examined_cells} examined · ${diagnosis.volatile_paths} volatile dependency paths`
        + (diagnosis.coverage_complete ? '' : ' · partial diagnostic coverage')));
    }
    const examples = (diagnosis.examples || []).filter(example => example.status === 'path_found');
    if (!examples.length) {
      root.append(make('p', '', 'No supported random/time dependency path is available. This does not prove absence of volatile dependencies.'));
      return;
    }
    const badge = make('p', 'formula-dependency-badge', formulaSummary(diagnosis));
    const select = make('select', '');
    select.setAttribute('aria-label', 'Cell dependency path');
    for (const example of examples) {
      const start = example.path[0];
      const end = example.path.at(-1);
      select.append(make('option', '', `${start.sheet}!${start.cell} → ${end.cell} · ${example.path.length} cells`));
    }
    const toggle = make('button', '', 'Pause formula pulse');
    toggle.type = 'button';
    const graph = make('div', 'formula-path');
    let pulsing = true;
    const render = () => {
      graph.replaceChildren();
      for (const [index, node] of examples[select.selectedIndex].path.entries()) {
        if (index) graph.append(make('span', 'formula-path-arrow', '→'));
        const card = make('article', 'formula-cell');
        card.append(make('strong', '', `${node.sheet}!${node.cell}`));
        card.append(make('p', 'formula-values', source
          ? `Source cached value: ${node.right_value ?? '(no cached value)'}`
          : `${region.left}: ${node.left_value ?? '(blank)'} → ${region.right}: ${node.right_value ?? '(blank)'}`));
        card.append(formulaCode(node, pulsing));
        if (node.shared_anchor) card.append(make('small', '', `Shared formula anchor: ${node.shared_anchor}`));
        if (!source && node.left_formula != null && node.left_formula !== node.formula) {
          card.append(make('p', '', `Left-state formula differs: ${node.left_formula}`));
        }
        graph.append(card);
      }
    };
    select.addEventListener('change', render);
    toggle.addEventListener('click', () => {
      pulsing = !pulsing;
      toggle.textContent = pulsing ? 'Pause formula pulse' : 'Resume formula pulse';
      render();
    });
    root.append(badge, select, toggle, graph);
    render();
  }).catch(error => { if (root.isConnected) note.textContent = `Formula evidence unavailable: ${error.message}`; });
  return root;
}
