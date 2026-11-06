import test from 'node:test';
import assert from 'node:assert/strict';
import { optionValue, setOption } from '../processor-options.js';

test('typed fields preserve paths with spaces and unrelated processor arguments', () => {
  const option = { name: 'rayon-threads', kind: 'integer' };
  const original = ['C:\\source files\\a.xlsx', '{output_dir}', '--rayon-threads=2', '--compression-level', '2'];
  assert.equal(optionValue(original, option), '2');
  const updated = setOption(original, option, 4);
  assert.deepEqual(updated, ['C:\\source files\\a.xlsx', '{output_dir}', '--compression-level', '2', '--rayon-threads', '4']);
  assert.equal(optionValue(updated, option), '4');
});

test('clearing options restores CLI defaults and repeated sheet names remain separate', () => {
  const sheets = { name: 'sheets', kind: 'list' };
  const original = ['{input}', '{output}', '--sheets', 'sheet1.xml', 'sheet2.xml', '--force-overwrite'];
  assert.equal(optionValue(original, sheets), 'sheet1.xml\nsheet2.xml');
  assert.deepEqual(setOption(original, sheets, ''), ['{input}', '{output}', '--force-overwrite']);
  assert.deepEqual(setOption(['{input}', '{output}'], sheets, 'sheet3.xml\nsheet4.xml'),
    ['{input}', '{output}', '--sheets', 'sheet3.xml', 'sheet4.xml']);
  assert.deepEqual(setOption(original, { name: 'force-overwrite', kind: 'boolean' }, false), original.slice(0, -1));
});
