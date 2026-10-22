'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const transport = require('../web/transport.js');

test('batch planning respects count and aggregate pixel budgets', () => {
  const tiles = Array.from({length: 8}, () => ({width: 8000, expectedHeight: 4000}));
  assert.equal(transport.batchEnd(tiles, 0, 4, 64000000), 2);
  assert.equal(transport.batchEnd(tiles, 2, 1, 64000000), 3);
  assert.equal(transport.batchEnd(tiles, 7, 4, 64000000), 8);
  assert.throws(() => transport.batchEnd(tiles, 0, 4, 31000000), /exceeds/);
});

test('small captures can fill a four-image batch', () => {
  const tiles = Array.from({length: 8}, () => ({width: 1920, expectedHeight: 1080}));
  assert.equal(transport.batchEnd(tiles, 0, 4, 64000000), 4);
});
