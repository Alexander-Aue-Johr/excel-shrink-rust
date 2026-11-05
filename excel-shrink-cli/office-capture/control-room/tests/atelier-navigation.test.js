import assert from 'node:assert/strict';
import { adjacentDifference, loadDifferentWorkbooks } from '../atelier-navigation.js';
const files = [{ id: 'a' }, { id: 'b' }];
assert.deepEqual(adjacentDifference(2, 0, files, 'a', 1), { regionIndex: 1 });
assert.deepEqual(adjacentDifference(2, 1, files, 'a', 1), { workbook: files[1], selectLast: false });
assert.deepEqual(adjacentDifference(2, 0, files, 'b', -1), { workbook: files[0], selectLast: true });
assert.equal(adjacentDifference(2, 0, files, 'a', -1), null);
assert.equal(adjacentDifference(2, 1, files, 'b', 1), null);
let calls = 0;
globalThis.fetch = async () => ({ ok: true, json: async () => ({ available: true,
  run_id: 'one', workbooks: [files[calls++]], next_offset: calls === 1 ? 100 : null }) });
assert.deepEqual(await loadDifferentWorkbooks(), files);
calls = 0;
globalThis.fetch = async () => ({ ok: true, json: async () => ({ available: true,
  run_id: String(calls++), workbooks: [], next_offset: 100 }) });
await assert.rejects(loadDifferentWorkbooks(), /run changed/);
console.log('PASS cross-file boundaries, pagination and run consistency');
