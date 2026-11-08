import assert from 'node:assert/strict';
import { formulaSegments, formulaSummary } from '../formula-diagnostics.js';
const formula = 'IF("🙂"="x",RAND(),0)';
const start = formula.indexOf('RAND');
assert.deepEqual(formulaSegments(formula, [start, start + 6]), ['IF("🙂"="x",', 'RAND()', ',0)']);
assert.deepEqual(formulaSegments('<script>', [-1, 3]), ['<script>', '', '']);
assert.match(formulaSummary({ volatile_paths: 2, evidence: 'prepared_source_cached_values' }), /captured values unavailable/);
assert.match(formulaSummary({ volatile_paths: 2, evidence: 'saved_compared_states' }), /other differences remain unclassified/);
assert.equal(formulaSummary({ volatile_paths: 0 }), '');
assert.match(formulaSummary({ volatile_paths: 0, environment_paths: 2 }), /2 filename\/environment dependency paths/);
assert.doesNotMatch(formulaSummary({ volatile_paths: 0, environment_paths: 2 }), /random\/time/);
console.log('PASS formula highlighting, UTF-16 spans and honest source/saved-state labels');

const { requestFormulaDiagnostics } = await import('../formula-diagnostics.js');
let calls = 0;
globalThis.fetch = async () => {
  calls++;
  return Response.json({ report_id: 'one', diagnosis: { evidence: 'saved_compared_states', volatile_paths: 2 } });
};
await requestFormulaDiagnostics('book', '0:findings:0', 'one');
await requestFormulaDiagnostics('book', '0:findings:0', 'one');
assert.equal(calls, 1, 'The same immutable region can reuse a resolved diagnostic');
await assert.rejects(requestFormulaDiagnostics('book', '0:findings:0', 'two'), /Comparison changed/);
assert.equal(calls, 2, 'A different report cannot reuse old formula evidence');

let release;
const blocked = new Promise(resolve => { release = resolve; });
globalThis.fetch = async () => {
  await blocked;
  return Response.json({ report_id: 'one', diagnosis: { evidence: 'saved_compared_states' } });
};
const requests = Array.from({ length: 16 }, (_, index) =>
  requestFormulaDiagnostics('bounded', `${index}:findings:0`, 'one'));
assert.equal(requestFormulaDiagnostics('bounded', '0:findings:0', 'one'), requests[0],
  'Repeated views share an in-flight request');
await assert.rejects(requestFormulaDiagnostics('bounded', '17:findings:0', 'one'), /queue is full/);
release();
await Promise.all(requests);
console.log('PASS formula cache identity and bounded in-flight requests');
