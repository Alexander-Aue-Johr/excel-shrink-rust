const assert = require('node:assert/strict');
const test = require('node:test');
const diagnosis = require('./resident-diagnosis.js');

test('capture packages upload distinct indexed receipts and omit zero-area images', async () => {
  globalThis.TargetedDiagnosis = {run: async () => ({images: [
    {range:'A1', image_base64:'YWJj'},
    {range:'A2', non_rendering:true, geometry:{height:0}},
    {range:'A3', image_base64:'YWJj'}
  ]})};
  globalThis.ResidentCapture = {encoded: async () => ({bytes:new Uint8Array([97,98,99])})};
  const uploaded = [];
  const result = await diagnosis.run({id:'batch',operation:'targeted',request:{kind:'capture_many'}}, {
    transport:{diagnosisImage:async (id, bytes, index) => {
      uploaded.push(index);
      return {image_path:`owned/${id}-${index}.image`};
    }}
  });
  assert.deepEqual(uploaded, [0,2]);
  assert(result.images.every(image => image.image_base64 === undefined));
  assert.equal(result.images[2].image_path, 'owned/batch-2.image');
});

test('a partial package upload cannot publish a successful image package', async () => {
  globalThis.TargetedDiagnosis = {run: async () => ({images:[
    {range:'A1',image_base64:'YWJj'},{range:'A2',image_base64:'YWJj'}
  ]})};
  globalThis.ResidentCapture = {encoded: async () => ({bytes:new Uint8Array([1])})};
  const result = await diagnosis.run({id:'batch',operation:'targeted',request:{kind:'capture_many'}}, {
    transport:{diagnosisImage:async (id, bytes, index) => {
      if (index === 1) throw new Error('Second upload failed');
      return {image_path:'owned/first.image'};
    }}
  });
  assert.match(result.diagnostic_error.message, /Second upload failed/);
  assert.equal(result.images, undefined);
});

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
