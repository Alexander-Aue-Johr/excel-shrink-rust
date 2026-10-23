'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const transport = require('./transport.js');

test('refinement splits in two axes where possible and never creates empty cells', () => {
  const quadrants = transport.split({r0: 0, r1: 5, c0: 0, c1: 7}, 't2');
  assert.deepEqual(quadrants, [
    {id: 't2.0', rect: {r0: 0, r1: 2, c0: 0, c1: 3}},
    {id: 't2.1', rect: {r0: 0, r1: 2, c0: 3, c1: 7}},
    {id: 't2.2', rect: {r0: 2, r1: 5, c0: 0, c1: 3}},
    {id: 't2.3', rect: {r0: 2, r1: 5, c0: 3, c1: 7}}
  ]);
  assert.equal(transport.split({r0: 2, r1: 3, c0: 4, c1: 5}, 'leaf').length, 0);
  assert.equal(transport.split({r0: 2, r1: 3, c0: 0, c1: 4}, 'row').length, 2);
});

test('first round sends hashes together, then refines and uploads one leaf', async () => {
  global.crypto = require('node:crypto').webcrypto;
  global.atob = value => Buffer.from(value, 'base64').toString('binary');
  const image = Buffer.from('a fake PNG for the transport boundary').toString('base64');
  global.CaptureGeometry = {
    SheetGeometry: class {
      constructor() { this.maxPixels = 32000000; this.fixedWidth = null; }
      async read() {}
      async calibrate() {}
      tiles() { return [{raw: {r0: 0, r1: 2, c0: 0, c1: 2},
        core: {r0: 0, r1: 2, c0: 0, c1: 2},
        cropY: 0, cropHeight: 2, width: 2, expectedHeight: 2}]; }
      imageWidth() { return 2; }
    },
    address: rect => `${rect.r0}:${rect.r1},${rect.c0}:${rect.c1}`,
    pngSize: () => ({width: 2, height: 2})
  };
  global.CaptureOverflow = {crop: async value => value};
  const calls = [];
  global.fetch = async (path, options) => {
    calls.push({path, options});
    return {ok: true, json: async () => ({ok: true})};
  };
  const send = async (path, body) => {
    calls.push({path, body});
    if (path === '/pair/geometry') return {tile_rows: [[0, 2]], capture_right_column: 1};
    if (path === '/round') return {divergent: [0]};
    if (path === '/refine') return {divergent: ['t0.3']};
    throw new Error(`Unexpected endpoint ${path}`);
  };
  const helpers = {
    bounds: () => ({bottom: 1, right: 1}), targetRows: () => null,
    checkedWorksheet: async () => {}, hideGridlines: async () => {},
    verifyDrawingBounds: async () => {},
    office: async action => action({workbook: {worksheets: {getItem: () => ({
      getRange: () => ({getImage: () => ({value: image})})
    })}}, sync: async () => {}})
  };
  transport.configure({run_id: 'original', side: 'original', batch_size: 4,
    max_batch_pixels: 100}, send, helpers);
  const sheet = {index: 1, name: 'Sheet', bottom_row: 1, base_right_column: 1,
    horizontal_overflow_required: false, drawing_fallback_required: false};
  await transport.prepareAll([sheet]);
  transport.begin();
  await transport.captureSheet(sheet);

  assert.equal(calls.filter(call => call.path === '/round').length, 1);
  assert.equal(calls.find(call => call.path === '/round').body.tiles.length, 1);
  assert.equal(calls.find(call => call.path === '/refine').body.parts.length, 4);
  assert.equal(calls.filter(call => call.path.startsWith('/evidence/')).length, 1);
  assert.equal(transport.metrics.first_round_tiles, 1);
  assert.equal(transport.metrics.refinement_tiles, 4);
  assert.equal(transport.metrics.evidence_images, 1);
});
