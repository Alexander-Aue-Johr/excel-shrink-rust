import test from 'node:test';
import assert from 'node:assert/strict';
import { configurationFromForm, partitionChoices } from '../study-configuration.js';

test('study fields retain independent typed worker and capture settings', () => {
  const defaults = {
    archive: '', conversion_processes: 1, prepare_processes: 1,
    geometry_batch_ranges: 256, capture_batch_ranges: 8, check_performance: false,
  };
  const fields = {
    archive: { value: ' G:\\Corpus files\\source.zip ' },
    conversion_processes: { value: '32' },
    prepare_processes: { value: '4' },
    geometry_batch_ranges: { value: '512' },
    capture_batch_ranges: { value: '3' },
    check_performance: { checked: true },
  };
  const form = { elements: { namedItem: name => fields[name] } };
  assert.deepEqual(configurationFromForm(form, defaults), {
    archive: 'G:\\Corpus files\\source.zip', conversion_processes: 32,
    prepare_processes: 4, geometry_batch_ranges: 512,
    capture_batch_ranges: 3, check_performance: true,
  });
  fields.capture_batch_ranges.value = '';
  assert.throws(() => configurationFromForm(form, defaults), /Missing integer/);
  fields.capture_batch_ranges.value = '2.5';
  assert.throws(() => configurationFromForm(form, defaults), /Invalid integer/);
  delete fields.capture_batch_ranges;
  assert.throws(() => configurationFromForm(form, defaults), /Missing study field/);
});

test('partition choices explicitly include the full corpus and every size part', () => {
  assert.deepEqual(partitionChoices(4), [0, 1, 2, 3, 4]);
  assert.equal(partitionChoices(1000).length, 1001);
  for (const count of [0, -1, 1.5, NaN, 1001]) {
    assert.throws(() => partitionChoices(count));
  }
});
