/** Verify the real HTTP-to-private-Excel path using a disposable prepared bundle. */
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { spawn, spawnSync } = require('node:child_process');
const { createHash } = require('node:crypto');
const repository = path.resolve(__dirname, '../../../..');
const root = path.resolve(process.argv[2]);
const base = 'http://127.0.0.1:8879';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

async function main() {
  const settings = path.join(root, 'host-settings.json');
  fs.writeFileSync(settings, JSON.stringify({ root: path.join(root, 'control'), port: 8879,
    web_root: path.join(repository, 'excel-shrink-cli/office-capture/control-room') }));
  const bundle = path.join(root, 'prepared/smoke.zip');
  const digest = () => createHash('sha256').update(fs.readFileSync(bundle)).digest('hex');
  const before = digest();
  const host = spawn(path.join(repository, 'target/debug/excel-shrink-cli.exe'), ['capture-host', 'control-room', settings],
    { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
  const errors = [];
  host.stderr.on('data', chunk => errors.push(String(chunk)));
  let request;
  try {
    let ready = false;
    for (let index = 0; index < 100; index++) {
      try { ready = (await fetch(base + '/health')).ok; } catch { }
      if (ready) break;
      await sleep(100);
    }
    assert.ok(ready, 'The isolated host starts.');
    const previous = await (await fetch(base + '/api/targeted-comparisons')).json();
    assert.ok(previous.comparisons.some(item => item.status === 'completed_with_limitations'));
    const response = await fetch(base + '/api/groups/smoke/targeted-compare', { method: 'POST', headers: { Origin: base } });
    const started = await response.json();
    assert.equal(started.ok, true, JSON.stringify(started));
    request = started.request_id;
    let result;
    for (let attempt = 0; attempt < 100; attempt++) {
      const snapshots = await (await fetch(base + '/api/targeted-comparisons')).json();
      result = snapshots.comparisons.find(item => !previous.comparisons.some(old => old.run_id === item.run_id));
      if (result?.finished_at) break;
      await sleep(300);
    }
    assert.equal(result?.status, 'completed_with_limitations');
    assert.equal(result.workbook, 'Synthetic font-height control.xlsx');
    assert.ok(result.lanes.every(lane => !lane.pid || lane.lifecycle === 'closed' && lane.process_alive === false));
    assert.equal(result.report.before_differences, 1);
    assert.equal(result.report.original_recalc_drift, 0);
    assert.equal(digest(), before, 'Prepared evidence remains immutable.');
    const silentHost = path.join(root, 'silent-host.ps1');
    fs.writeFileSync(silentHost, '$null = [Console]::ReadLine()\nStart-Sleep -Seconds 5\n');
    const timeoutOutput = path.join(root, `timeout-${Date.now()}`);
    const requestedAt = Date.now();
    const timeout = spawnSync(path.join(repository, 'target/debug/excel-shrink-cli.exe'), [
      'regression', 'targeted-compare', '--original', path.join(root, 'inputs/original.xlsx'),
      '--default', path.join(root, 'inputs/default.xlsx'), '--output', timeoutOutput,
      '--host-script', silentHost, '--operation-timeout-seconds', '1',
    ], { windowsHide: true, timeout: 10000 });
    assert.notEqual(timeout.status, 0);
    assert.ok(Date.now() - requestedAt < 10000, 'A silent command host has a bounded deadline.');
    const timedOut = JSON.parse(fs.readFileSync(path.join(timeoutOutput, 'telemetry.json'), 'utf8'));
    assert.equal(timedOut.status, 'failed');
    assert.ok(timedOut.finished_at && timedOut.lanes.every(lane => !lane.pid));
    fs.writeFileSync(path.join(root, 'api-check.json'), JSON.stringify({ passed: true, request_id: request,
      private_comparison: result, prepared_bundle_unchanged: true, command_timeout_verified: true }, null, 2));
    console.log('Targeted HTTP launch, real captures, immutable inputs, process cleanup and command deadline passed.');
  } finally {
    // Stop only the launcher owned by this fixture if a failed check left it running.
    if (request) {
      const resultPath = path.join(root, 'control/targeted-runs', request, 'launcher-result.json');
      for (let attempt = 0; attempt < 20 && !fs.existsSync(resultPath); attempt++) await sleep(100);
      if (!fs.existsSync(resultPath)) {
        console.error(`Fixture launcher may still be running: ${request}; inspect its own launch logs.`);
      }
    }
    host.kill();
    fs.writeFileSync(path.join(root, 'host.stderr.log'), errors.join(''));
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
