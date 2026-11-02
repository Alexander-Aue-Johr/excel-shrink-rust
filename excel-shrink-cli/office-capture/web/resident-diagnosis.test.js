const assert = require('node:assert/strict');
const test = require('node:test');
const diagnosis = require('./resident-diagnosis.js');

test('targeted captures upload binary bytes and publish only the receipt', async () => {
  globalThis.TargetedDiagnosis = {run: async () => ({image_base64: 'YWJj', geometry: {width: 10}})};
  globalThis.ResidentCapture = {encoded: async (base64, hash) => {
    assert.equal(hash, false);
    return {bytes: new Uint8Array(Buffer.from(base64, 'base64'))};
  }};
  const result = await diagnosis.run({id: 'capture-1', operation: 'targeted', request: {kind: 'capture'}}, {
    transport: {diagnosisImage: async (id, bytes) => {
      assert.equal(id, 'capture-1');
      assert.deepEqual([...bytes], [97, 98, 99]);
      return {image_path: 'owned/session/capture-1.image', sha256: 'receipt'};
    }}
  });
  assert.equal(result.image_base64, undefined);
  assert.equal(result.image_path, 'owned/session/capture-1.image');
  assert.deepEqual(result.geometry, {width: 10});
});

test('a failed upload cannot publish successful capture evidence', async () => {
  globalThis.TargetedDiagnosis = {run: async () => ({image_base64: 'YWJj'})};
  const result = await diagnosis.run({id: 'capture-2', operation: 'targeted', request: {kind: 'capture'}}, {
    transport: {diagnosisImage: async () => {throw new Error('Upload failed');}}
  });
  assert.match(result.diagnostic_error.message, /Upload failed/);
  assert.equal(result.image_path, undefined);
});

test('zero-area chart observations do not upload invented pixels', async () => {
  globalThis.TargetedDiagnosis = {run: async () => ({non_rendering: true, reason: 'zero-area-chart'})};
  const result = await diagnosis.run({id: 'capture-3', operation: 'targeted', request: {kind: 'capture_chart'}}, {});
  assert.equal(result.non_rendering, true);
  assert.equal(result.image_path, undefined);
});
