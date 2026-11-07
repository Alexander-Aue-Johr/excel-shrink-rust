const assert = require('node:assert/strict');
const test = require('node:test');
require('./targeted-diagnosis.js');

test('chart inventory reads live geometry without rendering any images', async () => {
  let syncs = 0;
  const chart = {name: 'Chart 1', top: 10, left: 20, width: 400, height: 200,
    getImage() {assert.fail('Inventory must not render');}};
  const sheets = {items: [{name: 'Data', charts: {items: [chart], load(properties) {
    assert.equal(syncs, 1);
    assert.equal(properties, 'items/name,items/top,items/left,items/width,items/height');
  }}}], load() {assert.equal(syncs, 0);}};
  const context = {workbook: {worksheets: sheets}, sync: async () => {syncs++;}};
  const result = await globalThis.TargetedDiagnosis.run({kind: 'charts'}, action => action(context));
  assert.equal(syncs, 2);
  assert.deepEqual(result.charts, [{sheet: 'Data', name: 'Chart 1', index: 0,
    geometry: {top: 10, left: 20, width: 400, height: 200}}]);
});

test('multiple captures share two syncs and preserve zero-area observations and order', async () => {
  let syncs = 0;
  const reads = [];
  const context = {
    workbook: {worksheets: {getItem: () => ({getRange: address => ({
      top: 0, left: 0, width: 100, height: address === 'A2' ? 0 : 100,
      load() { assert.equal(syncs, 0); },
      getImage() {
        assert.equal(syncs, 1);
        reads.push(address);
        return {get value() {assert.equal(syncs, 2); return address;}};
      }
    })})}},
    sync: async () => { syncs++; }
  };
  const result = await globalThis.TargetedDiagnosis.run({kind: 'capture_many', sheet: 'Data',
    ranges: ['A1', 'A2', 'A3'].map(range => ({range}))}, action => action(context));
  assert.equal(syncs, 2);
  assert.deepEqual(reads, ['A1', 'A3']);
  assert.deepEqual(result.images.map(image => image.range), ['A1', 'A2', 'A3']);
  assert.equal(result.images[1].non_rendering, true);
  assert.equal(result.images[2].image_base64, 'A3');
});

test('oversized combined captures fail before queuing any images', async () => {
  const context = {workbook: {worksheets: {getItem: () => ({getRange: () => ({
    top: 0, left: 0, width: 2500, height: 2500, load() {},
    getImage() {assert.fail('Combined budget must be checked first');}
  })})}}, sync: async () => {}};
  await assert.rejects(globalThis.TargetedDiagnosis.run({kind: 'capture_many', sheet: 'Data',
    ranges: [{range:'A1:P128'}, {range:'A129:P256'}]}, action => action(context)), /combined pixel budget/);
});

test('explicit package budget permits multiple individually bounded large images', async () => {
  let images = 0;
  const context = {workbook: {worksheets: {getItem: () => ({getRange: () => ({
    top: 0, left: 0, width: 2500, height: 2500, load() {},
    getImage() {images++; return {value: 'image'};}
  })})}}, sync: async () => {}};
  const request = {kind: 'capture_many', sheet: 'Data', batch_megapixels: 64,
    ranges: [{range: 'A1'}, {range: 'A2'}]};
  await globalThis.TargetedDiagnosis.run(request, action => action(context));
  assert.equal(images, 2);
  for (const invalid of [0, 31, 257, 32.5, '64']) {
    await assert.rejects(globalThis.TargetedDiagnosis.run({...request, batch_megapixels: invalid},
      action => action(context)), /package budget/);
  }
});

test('geometry batches load every range before one synchronization', async () => {
  let synchronized = false;
  let synchronizations = 0;
  const loaded = [];
  const addresses = Array.from({length: 64}, (_, index) => `A${index + 1}:P${index + 1}`);
  const context = {
    workbook: {worksheets: {getItem: sheet => {
      assert.equal(sheet, 'Data');
      return {getRange: address => ({
        load(properties) {
          assert.equal(properties, 'top,left,width,height');
          assert.equal(synchronized, false);
          loaded.push(address);
        },
        get top() { assert.equal(synchronized, true); return 15; },
        left: 0, width: 300, height: 0
      })};
    }}},
    sync: async () => {
      assert.deepEqual(loaded, addresses);
      synchronizations++;
      synchronized = true;
    }
  };
  const result = await globalThis.TargetedDiagnosis.run(
    {kind: 'measure_batch', sheet: 'Data', ranges: addresses}, action => action(context));
  assert.equal(synchronizations, 1);
  assert.deepEqual(result.ranges.map(item => item.range), addresses);
  assert(result.ranges.every(item => item.height === 0 && item.top === 15));
});

test('invalid geometry batches never load ranges or synchronize', async () => {
  const context = {workbook: {worksheets: {getItem() {assert.fail('Invalid batch');}}},
    sync() {assert.fail('Invalid batch');}};
  for (const ranges of [[], [''], [null], Array(4097).fill('A1')]) {
    await assert.rejects(globalThis.TargetedDiagnosis.run(
      {kind: 'measure_batch', sheet: 'Data', ranges}, action => action(context)), /1\.\.4096/);
  }
});

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


test('linked-workbook deadline counts continuous Pending time and leaves active calculation alone', async () => {
  const originalNow = Date.now;
  const originalTimeout = globalThis.setTimeout;
  const originalExcel = globalThis.Excel;
  try {
    globalThis.Excel = {
      CalculationType: {fullRebuild: 'FullRebuild'},
      CalculationState: {done: 'Done', pending: 'Pending', calculating: 'Calculating'},
      SaveBehavior: {save: 'Save'}
    };
    globalThis.setTimeout = callback => { callback(); return 0; };
    const cases = [
      {name: 'blocked links', links: 2, states: ['Pending'], rejected: true},
      {name: 'active calculation', links: 2, states: [...Array(40).fill('Calculating'), 'Done']},
      {name: 'pending interval resets', links: 2, states: [...Array(20).fill('Pending'), 'Calculating', ...Array(20).fill('Pending'), 'Done']},
      {name: 'no external-link evidence', links: 0, states: [...Array(40).fill('Pending'), 'Done']},
      {name: 'Office.js blocked by opening', links: 2, interval: 10000, states: ['Pending', 'Pending', 'Pending', 'Done']}
    ];
    for (const scenario of cases) {
      let now = 0;
      let saved = false;
      const states = [...scenario.states];
      let state = states[0];
      Date.now = () => now;
      const application = {
        calculate() {},
        load() { state = states.shift() || state; this.calculationState = state; }
      };
      const context = {
        workbook: {application, save() { saved = true; }},
        sync: async () => { now += scenario.interval || 500; }
      };
      const operation = globalThis.TargetedDiagnosis.run({kind: 'recalculate',
        external_link_parts: scenario.links, linked_pending_timeout_ms: 15000},
        action => action(context));
      if (scenario.rejected) {
        await assert.rejects(operation, error => error.code === 'RebuildPendingExternalLinks', scenario.name);
        assert.equal(saved, false);
      } else {
        assert.equal((await operation).saved, true, scenario.name);
        assert.equal(saved, true);
      }
    }
  } finally {
    Date.now = originalNow;
    globalThis.setTimeout = originalTimeout;
    globalThis.Excel = originalExcel;
  }
});
