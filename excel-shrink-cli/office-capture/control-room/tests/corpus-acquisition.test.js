import test from 'node:test';
import assert from 'node:assert/strict';
import { acquisitionRequest, acquisitionStatus, installCorpusAcquisition } from '../corpus-acquisition.js';

test('acquisition keeps explicit paths and operation without starting a study', () => {
  const fields = {
    profile: 'datagov', artifacts: ' V:/artifacts/datagov ',
    cache: ' V:/downloads ', archive: ' V:/offline/source.zip ',
  };
  const form = { elements: { namedItem: name => ({ value: fields[name] }) } };
  assert.deepEqual(acquisitionRequest(form, 'prepare'), {
    profile: 'datagov', operation: 'prepare', artifacts: 'V:/artifacts/datagov',
    cache: 'V:/downloads', archive: 'V:/offline/source.zip',
  });
  fields.archive = '';
  assert.equal(acquisitionRequest(form, 'inspect').archive, null);
  assert.throws(() => acquisitionRequest(form, 'start-study'), /operation/);
  fields.cache = ' ';
  assert.throws(() => acquisitionRequest(form, 'prepare'), /download cache/);
});

test('acquisition status distinguishes phases from terminal results', () => {
  assert.equal(acquisitionStatus(null), '');
  const job = {
    request: { profile: 'destatis' }, status: 'running',
    progress: { phase: 'verifying_archive', archive_bytes: 100 },
  };
  assert.match(acquisitionStatus(job), /running · verifying archive/);
  job.progress = { phase: 'extracting', added: 2, total: 10, member: 'a.xlsx' };
  assert.match(acquisitionStatus(job), /2 restored of 10 files · a.xlsx/);
  job.status = 'failed';
  assert.match(acquisitionStatus(job), /failed/);
});

test('profile controls preserve custom storage and render an independently running job', async () => {
  const element = (value = '') => ({
    value, listeners: new Map(),
    addEventListener(name, listener) { this.listeners.set(name, listener); },
    replaceChildren(...options) { this.value = options[0]?.value || ''; },
  });
  const ids = Object.fromEntries([
    'corpus-acquisition', 'corpus-profile', 'corpus-cache', 'corpus-artifacts',
    'corpus-profile-details', 'corpus-archive', 'corpus-fields', 'corpus-observation',
    'corpus-job-status', 'corpus-job-result',
  ].map(id => [id, element()]));
  ids['corpus-acquisition'].elements = {
    namedItem: name => ids[`corpus-${name}`],
  };
  const previousDocument = globalThis.document;
  globalThis.document = { getElementById: id => ids[id], createElement: () => element() };
  const profiles = ['datagov', 'destatis'].map(id => ({
    id, name: id, workbook_count: 3,
    archive: { bytes: 1024, url: 'https://example.org/archive.zip', sha256: 'a'.repeat(64) },
  }));
  const calls = [];
  const api = async (url, body) => {
    calls.push({ url, body });
    if (url === '/api/corpora') return { profiles };
    return { job: { request: body, status: 'running', progress: { phase: 'checking_originals' } } };
  };
  try {
    const view = await installCorpusAcquisition({
      api, message() {}, session: { corpus_artifacts_root: 'V:/artifacts/', corpus_download_cache: 'V:/cache' },
    });
    assert.equal(ids['corpus-artifacts'].value, 'V:/artifacts/datagov');
    ids['corpus-artifacts'].value = 'R:/custom-originals';
    ids['corpus-profile'].value = 'destatis';
    ids['corpus-profile'].listeners.get('change')();
    assert.equal(ids['corpus-artifacts'].value, 'R:/custom-originals');
    await ids['corpus-acquisition'].listeners.get('submit')({
      preventDefault() {}, submitter: { value: 'inspect' },
    });
    assert.equal(calls[1].url, '/api/corpora/start');
    assert.equal(calls[1].body.profile, 'destatis');
    assert.equal(ids['corpus-fields'].disabled, true);
    view.render({ request: { profile: 'destatis' }, status: 'completed', result: { verified: 3 } }, false);
    assert.equal(ids['corpus-fields'].disabled, false);
    assert.match(ids['corpus-job-result'].textContent, /"verified": 3/);
  } finally {
    globalThis.document = previousDocument;
  }
});
