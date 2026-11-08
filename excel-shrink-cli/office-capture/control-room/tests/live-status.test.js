import assert from 'node:assert/strict';
import { applyLivePacket } from '../live-status.js';

const id = '1794182130052-c8227113-adc4-4227-9396-167511a346c0';
const initial = applyLivePacket(null, { sequence: 1,
  snapshot: { queue: {}, comparisons: { [id]: { status: 'running' } } } });
const next = applyLivePacket(initial, { sequence: 2, patch: [
  { op: 'replace', path: `/comparisons/${id}`, value: { status: 'complete' } },
  { op: 'replace', path: '/queue', value: { running: true } },
] });
assert.equal(next.value.comparisons[id].status, 'complete');
assert.equal(initial.value.comparisons[id].status, 'running');
assert.equal(next.value.queue.running, true);
assert.throws(() => applyLivePacket(next, { sequence: 4, patch: [] }), /revision gap/);
assert.throws(() => applyLivePacket(next, { sequence: 3,
  patch: [{ op: 'add', path: '/comparisons/__proto__', value: {} }] }), /Unsupported live path/);
const removed = applyLivePacket(next, { sequence: 3,
  patch: [{ op: 'remove', path: `/comparisons/${id}` }] });
assert.equal(Object.keys(removed.value.comparisons).length, 0);
assert.equal(applyLivePacket(removed, { sequence: 1, snapshot: { comparisons: {} } }).sequence, 1);
console.log('PASS live baseline, keyed patch, removal, immutable prior state and resynchronization');
