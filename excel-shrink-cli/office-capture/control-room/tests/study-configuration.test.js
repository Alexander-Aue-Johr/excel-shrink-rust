import test from 'node:test';
import assert from 'node:assert/strict';
import { configurationFromForm, partitionChoices, prepareStudySource } from '../study-configuration.js';

test('study fields retain independent typed worker and capture settings', () => {
  const defaults = {
    archive: '', conversion_processes: 1, prepare_processes: 1,
    geometry_batch_ranges: 256, capture_batch_ranges: 8, check_performance: false,
    reuse_webview_profiles: false, warm_workers: true,
  };
  const fields = {
    archive: { value: ' G:\\Corpus files\\source.zip ' },
    conversion_processes: { value: '32' },
    prepare_processes: { value: '4' },
    geometry_batch_ranges: { value: '512' },
    capture_batch_ranges: { value: '3' },
    check_performance: { checked: true },
    reuse_webview_profiles: { checked: true },
    warm_workers: { checked: false },
  };
  const form = { elements: { namedItem: name => fields[name] } };
  assert.deepEqual(configurationFromForm(form, defaults), {
    archive: 'G:\\Corpus files\\source.zip', conversion_processes: 32,
    prepare_processes: 4, geometry_batch_ranges: 512,
    capture_batch_ranges: 3, check_performance: true,
    reuse_webview_profiles: true, warm_workers: false,
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

test('profile study waits for verified originals and released operation admission', async () => {
  const configuration = { corpus_profile: 'datagov', originals_root: 'V:/originals', corpus_cache: 'V:/cache', archive: '' };
  const calls = [];
  const statuses = [];
  const request = { profile: 'datagov' };
  let polls = 0;
  const api = async (url, body) => {
    calls.push({ url, body });
    if (url === '/api/corpora/start') return { job: { id: 'job', request, status: 'running' } };
    polls += 1;
    return { active: polls === 1, corpus_job: { id: 'job', request, status: 'completed', result: { inspection: { complete: true } } } };
  };
  await prepareStudySource(api, configuration, status => statuses.push(status));
  assert.deepEqual(calls[0].body, {
    profile: 'datagov', operation: 'prepare', artifacts: 'V:/originals', cache: 'V:/cache', archive: null,
  });
  assert.equal(calls[1].url, '/api/state');
  assert.equal(polls, 2);
  assert.match(statuses.at(-1), /completed/);
  assert.equal(calls.some(call => call.url === '/api/study/start'), false);
});

test('study source failures and incomplete results prevent handoff', async () => {
  const configuration = { corpus_profile: 'destatis' };
  await assert.rejects(prepareStudySource(async () => ({ job: {
    id: 'failed', request: { profile: 'destatis' }, status: 'failed', error: 'Local original changed',
  } }), configuration, () => {}), /Local original changed/);
  await assert.rejects(prepareStudySource(async () => ({ job: {
    id: 'incomplete', request: { profile: 'destatis' }, status: 'completed', result: { inspection: { complete: false } },
  } }), configuration, () => {}), /incomplete/);
  let calls = 0;
  await prepareStudySource(async () => { calls += 1; }, { corpus_profile: '' }, () => {});
  assert.equal(calls, 0);
});
