import { formulaSummary, renderFormulaDiagnosis } from './formula-inspector.js';
export { formulaSummary, formulaSegments } from './formula-inspector.js';
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

export function attachFormulaSummary(element, id) {
  element.dataset.formulaGroup = id;
  element.className = 'workbook-formula-summary';
  element.textContent = summaries.get(id) || '';
}

function remember(id, diagnosis) {
  if (!diagnosis.volatile_paths && !diagnosis.environment_paths) return;
  summaries.set(id, formulaSummary(diagnosis));
  for (const element of document.querySelectorAll('.workbook-formula-summary')) {
    if (element.dataset.formulaGroup === id) element.textContent = formulaSummary(diagnosis);
  }
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
    renderFormulaDiagnosis(root, region, diagnosis);
  }).catch(error => { if (root.isConnected) note.textContent = `Formula evidence unavailable: ${error.message}`; });
  return root;
}
