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

test('rebuild telemetry distinguishes dispatch, calculation wait and saving', async () => {
  globalThis.Excel = {CalculationType: {fullRebuild: 'FullRebuild'},
    CalculationState: {done: 'Done'}, SaveBehavior: {save: 'Save'}};
  const stages = [];
  const context = {workbook: {
    application: {calculate() {}, load() {}, calculationState: 'Done'},
    save() {}
  }, sync: async () => {}};
  const result = await globalThis.TargetedDiagnosis.run(
    {kind: 'recalculate'}, action => action(context), stage => stages.push(stage));
  assert.deepEqual(stages, ['rebuild_dispatch', 'rebuild_waiting_for_calculation',
    'saving_workbook', 'workbook_saved']);
  assert.equal(result.saved, true);
});

test('a blocked save does not publish completion', async () => {
  const stages = [];
  const context = {workbook: {save() {throw new Error('Save blocked');}}, sync: async () => {}};
  await assert.rejects(globalThis.TargetedDiagnosis.run(
    {kind: 'save'}, action => action(context), stage => stages.push(stage)), /Save blocked/);
  assert.deepEqual(stages, ['saving_workbook']);
});


test('duplicate chart names retain distinct indices and export the requested object', async () => {
  const items = [0, 1].map(index => ({
    name: 'Chart 1', top: index * 200, left: 0, width: 100, height: 80,
    load() {}, getImage() { return {value: `image-${index}`}; }
  }));
  const charts = {
    items, load() {}, getItemAt: index => items[index],
    getItem() { assert.fail('Name lookup would select the first duplicate'); }
  };
  const sheet = {name: 'Summary', charts};
  const context = {
    workbook: {worksheets: {items: [sheet], load() {}, getItem: () => sheet}},
    sync: async () => {}
  };
  const inventory = await globalThis.TargetedDiagnosis.run(
    {kind: 'charts'}, action => action(context));
  assert.deepEqual(inventory.charts.map(chart => [chart.name, chart.index]),
    [['Chart 1', 0], ['Chart 1', 1]]);
  const result = await globalThis.TargetedDiagnosis.run(
    {kind: 'capture_chart', sheet: 'Summary', chart: 'Chart 1', chart_index: 1},
    action => action(context));
  assert.equal(result.image_base64, 'image-1');
  assert.equal(result.geometry.top, 200);
  await assert.rejects(globalThis.TargetedDiagnosis.run(
    {kind: 'capture_chart', sheet: 'Summary', chart: 'Renamed', chart_index: 1},
    action => action(context)), /order changed/);
  await assert.rejects(globalThis.TargetedDiagnosis.run(
    {kind: 'capture_chart', sheet: 'Summary', chart: 'Chart 1', chart_index: -1},
    action => action(context)), /Invalid chart collection index/);
});
