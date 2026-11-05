import assert from 'node:assert/strict';
import { readDiagnosticResponse } from '../diagnostic-response.js';

assert.deepEqual(await readDiagnosticResponse(Response.json({ regions: [] }), 'Pixels'), { regions: [] });
await assert.rejects(readDiagnosticResponse(new Response(null, { status: 404 }), 'Pixels'), error => {
  assert.equal(error.code, 'unsupported-diagnostic-endpoint');
  assert.match(error.message, /older server/);
  assert.match(error.message, /current run/);
  return true;
});
await assert.rejects(readDiagnosticResponse(Response.json({ error: 'No retained report' }, { status: 404 }), 'Pixels'),
  error => error.message === 'No retained report' && !error.code);
await assert.rejects(readDiagnosticResponse(new Response('{'), 'Formulas'), /invalid server JSON response/);
await assert.rejects(readDiagnosticResponse(new Response(''), 'Formulas'), /empty server response/);
await assert.rejects(readDiagnosticResponse(new Response('<html>proxy error</html>', { status: 502 }), 'Pixels'), /HTTP 502/);
console.log('PASS diagnostic responses distinguish unsupported routes, unavailable evidence and invalid replies');
