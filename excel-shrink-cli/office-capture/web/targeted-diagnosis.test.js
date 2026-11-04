const assert = require('node:assert/strict');
const test = require('node:test');
require('./targeted-diagnosis.js');

test('capture checks visible geometry before rendering', async () => {
  let synchronizations = 0;
  const range = {
    top: 10, left: 20, width: 300, height: 400,
    load() {},
    getImage() { return {value: 'encoded-image'}; }
  };
  const context = {
    workbook: {worksheets: {getItem: () => ({getRange: () => range})}},
    sync: async () => { synchronizations++; }
  };
  const result = await globalThis.TargetedDiagnosis.run(
    {kind: 'capture', sheet: 'Data', range: 'A1:P128'}, action => action(context));
  assert.equal(synchronizations, 2);
  assert.equal(result.image_base64, 'encoded-image');
  assert.deepEqual(result.geometry, {top: 10, left: 20, width: 300, height: 400});
});

test('a zero-height chart records geometry without calling the failing image API', async () => {
  const chart = {
    top: 350.25, left: 176.66, width: 1072.7, height: 0,
    load() {},
    getImage() { assert.fail('Zero-area charts have no image to export'); }
  };
  const context = {
    workbook: {worksheets: {getItem: () => ({charts: {getItem: () => chart}})}},
    sync: async () => {}
  };
  const result = await globalThis.TargetedDiagnosis.run(
    {kind: 'capture_chart', sheet: 'Data', chart: 'Chart 11'}, action => action(context));
  assert.equal(result.non_rendering, true);
  assert.equal(result.reason, 'zero-area-chart');
  assert.equal(result.geometry.height, 0);
  assert.equal(result.image_base64, undefined);
});

test('a zero-height cell range is observed without exporting an image', async () => {
  const range = {top: 15, left: 0, width: 866.25, height: 0,
    load() {}, getImage() { assert.fail('Hidden ranges must not be exported'); }};
  const context = {workbook: {worksheets: {getItem: () => ({getRange: () => range})}},
    sync: async () => {}};
  const result = await globalThis.TargetedDiagnosis.run(
    {kind: 'capture', sheet: 'Data', range: 'A129:P256'}, action => action(context));
  assert.equal(result.reason, 'zero-area-range');
  assert.equal(result.geometry.height, 0);
  assert.equal(result.image_base64, undefined);
});


test('capture measures its unpadded core without extra synchronization and disables gridlines', async () => {
  let synchronizations = 0;
  const core = {top:0,left:0,width:280,height:380,load() {}};
  const range = {top:0,left:0,width:300,height:400,load() {},
    getImage() { assert.equal(worksheet.showGridlines,false); return {value:'png'}; }};
  const worksheet = {showGridlines:true,getRange: address => address === 'A1:H6' ? core : range};
  const context = {workbook:{worksheets:{getItem:()=>worksheet}},
    sync:async()=>{synchronizations++;}};
  const result = await globalThis.TargetedDiagnosis.run({kind:'capture',sheet:'Deals',
    range:'A1:I7',comparison_core_range:'A1:H6'},action=>action(context));
  assert.equal(synchronizations,2);
  assert.deepEqual(result.geometry.comparison_core,{top:0,left:0,width:280,height:380});
});
