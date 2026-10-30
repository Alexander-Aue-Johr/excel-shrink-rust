'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const {sheetSweepProgress} = require('../web/resident.js');

test('one process counts completed sheets across both hash passes', () => {
  const sheets = [{index: 1, name: 'First'}, {index: 2, name: 'Second'}, {index: 3, name: 'Third'}];
  const command = {id: '1', sheets, progress_pass: 1, progress_passes: 2};
  const first = sheetSweepProgress(command, 0, sheets[0]);
  assert.equal(first.completed, 0);
  assert.equal(first.total, 6);
  assert.equal(sheetSweepProgress(command, 1, sheets[0]).completed, 1);
  const next = sheetSweepProgress(command, 1, sheets[1]);
  assert.equal(next.completed, 1);
  assert.equal(next.sheet_name, 'Second');
  assert.equal(sheetSweepProgress(command, 3, sheets[2]).completed, 3);
  command.id = '2';
  command.progress_pass = 2;
  assert.equal(sheetSweepProgress(command, 0, sheets[0]).completed, 3);
  assert.equal(sheetSweepProgress(command, 1, sheets[1]).completed, 4);
  assert.equal(sheetSweepProgress(command, 3, sheets[2]).completed, 6);
});

test('a single capture pass counts sheets without inventing another pass', () => {
  const command = {id: '1', sheets: [{index: 4, name: 'Only'}]};
  const done = sheetSweepProgress(command, 1, command.sheets[0]);
  assert.equal(done.completed, 1);
  assert.equal(done.total, 1);
  assert.equal(done.passes, 1);
});

test('sheet changes reuse the same DOM row, track, fill and label', async () => {
  class Node {
    constructor(tag) { this.tag = tag; this.children = []; this.style = {}; this.attributes = {}; }
    append(...nodes) { this.children.push(...nodes); }
    setAttribute(key, value) { this.attributes[key] = value; }
    removeAttribute(key) { delete this.attributes[key]; }
  }
  global.document = {createElement: tag => new Node(tag), addEventListener() {}};
  global.matchMedia = () => ({matches: false, addEventListener() {}});
  global.IntersectionObserver = class { observe() {} unobserve() {} disconnect() {} };
  const {createCaptureProgressRow} = await import('../control-room/worker-monitor.js');
  const {captureProcessKey} = await import('../control-room/worker-monitor-model.js');
  const worker = {group_id: 'group', attempt_id: 'attempt', profile: 'default', pid: 42,
    process_started_unix_ms: 123, workbook: 'Book', sheet_name: 'First', completed: 1,
    total: 6, unit: 'sheet passes', pass: 1, passes: 2};
  const view = createCaptureProgressRow();
  view.update(worker);
  const row = view.element, track = row.children[1];
  const fill = track.children[0], label = track.children[1];
  const identity = captureProcessKey(worker);
  view.update({...worker, sheet_name: 'Second', completed: 2});
  assert.equal(view.element, row);
  assert.equal(row.children[1], track);
  assert.equal(track.children[0], fill);
  assert.equal(track.children[1], label);
  assert.equal(track.children.length, 2);
  assert.match(label.textContent, /Second.*2 \/ 6 sheet passes/);
  assert.equal(track.attributes['aria-valuenow'], '2');
  assert.equal(captureProcessKey({...worker, sheet_name: 'Second', command_id: 'new'}), identity);
  assert.notEqual(captureProcessKey({...worker, pid: 43}), identity);
  delete global.document;
  delete global.matchMedia;
  delete global.IntersectionObserver;
});
