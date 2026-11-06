import assert from 'node:assert/strict';
import { DiagnosticCache } from '../diagnostic-cache.js';

const cache = new DiagnosticCache({ maxEntries: 2, maxBytes: 200 });
cache.set('first', { value: 1 });
cache.set('second', { value: 2 });
assert.deepEqual(cache.get('first'), { value: 1 });
cache.set('third', { value: 3 });
assert.equal(cache.get('second'), undefined, 'Reading an entry renews its LRU position');
assert.deepEqual(cache.get('first'), { value: 1 });
cache.set('large', 'x'.repeat(200));
assert.equal(cache.get('large'), undefined, 'An oversized graph is not retained');
assert.deepEqual(cache.get('first'), { value: 1 }, 'Oversized responses do not evict useful small entries');
cache.set('first', { value: 4 });
assert.deepEqual(cache.get('first'), { value: 4 }, 'A rerun replaces earlier diagnostic evidence');
for (let index = 0; index < 9000; index++) cache.set(String(index), { value: index });
assert.equal(cache.entries.size, 2, 'Cache size is independent of elapsed run length');
assert.ok(cache.bytes <= 200);
cache.delete('8999');
cache.delete('8998');
assert.equal(cache.bytes, 0, 'Eviction and removal release the entire accounting budget');

const disabled = new DiagnosticCache({ maxEntries: 0 });
disabled.set('one', {});
assert.equal(disabled.get('one'), undefined);
console.log('PASS bounded diagnostic retention, replacement and LRU eviction');
