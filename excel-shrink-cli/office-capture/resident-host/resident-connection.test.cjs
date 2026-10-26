const test = require('node:test');
const assert = require('node:assert/strict');
const {createJobConnection, postResult, runJobs} = require('../web/resident-connection.js');

const flush = () => new Promise(resolve => setImmediate(resolve));

function harness() {
  let clock = 0;
  let timerId = 0;
  const timers = new Map();
  const sockets = [];
  const statuses = [];
  const environment = {
    now: () => clock,
    setTimeout(action, delay) {
      const id = ++timerId;
      timers.set(id, {action, due: clock + delay});
      return id;
    },
    clearTimeout(id) {timers.delete(id);},
    createSocket(url) {
      assert.equal(url, 'ws://localhost/events');
      const socket = {
        readyState: 0,
        sent: [],
        send(value) {this.sent.push(JSON.parse(value));},
        open() {this.readyState = 1; this.onopen();},
        receive(message) {this.onmessage({data: JSON.stringify(message)});},
        close(code = 1000) {
          this.readyState = 3;
          this.onclose?.({code, reason: ''});
        }
      };
      sockets.push(socket);
      return socket;
    }
  };
  const jobs = createJobConnection({url: 'ws://localhost/events', client: 'original', token: 'secret',
    connection: (...value) => statuses.push(value)}, environment);
  function tick(milliseconds) {
    const end = clock + milliseconds;
    while (true) {
      const due = [...timers.entries()].filter(([, timer]) => timer.due <= end)
        .sort((left, right) => left[1].due - right[1].due)[0];
      if (!due) break;
      timers.delete(due[0]);
      clock = due[1].due;
      due[1].action();
    }
    clock = end;
  }
  function open(index = sockets.length - 1) {
    sockets[index].open();
    sockets[index].receive({type: 'changed', revision: 0});
    return sockets[index];
  }
  return {jobs, sockets, timers, statuses, tick, open};
}

test('idle workers ask once and then sleep until a host change', async () => {
  const f = harness();
  const socket = f.open();
  assert.deepEqual(socket.sent, [{role: 'worker', client: 'original', token: 'secret'}]);
  const pending = f.jobs.next();
  socket.receive({type: 'idle', revision: 0});
  f.tick(60000);
  assert.equal(socket.sent.filter(message => message.type === 'next').length, 1);
  socket.receive({type: 'changed', revision: 0});
  assert.equal(socket.sent.filter(message => message.type === 'next').length, 1);
  socket.receive({type: 'changed', revision: 1});
  assert.equal(socket.sent.filter(message => message.type === 'next').length, 2);
  socket.receive({type: 'job', command: {id: '000001', kind: 'plan'}});
  assert.equal((await pending).id, '000001');
  f.jobs.close();
  assert.equal(f.timers.size, 0);
});

test('a change arriving during dispatch is reconciled without a lost wakeup', async () => {
  const f = harness();
  const socket = f.open();
  const pending = f.jobs.next();
  socket.receive({type: 'changed', revision: 1});
  socket.receive({type: 'idle', revision: 0});
  assert.equal(socket.sent.filter(message => message.type === 'next').length, 2);
  socket.receive({type: 'job', command: {id: '000001'}});
  await pending;
  f.jobs.close();
});

test('reconnect preserves the waiter, uses the same token and ignores old socket messages', async () => {
  const f = harness();
  const old = f.open();
  const pending = f.jobs.next();
  old.close(1006);
  f.tick(250);
  const replacement = f.open();
  assert.deepEqual(replacement.sent[0], {role: 'worker', client: 'original', token: 'secret'});
  old.receive({type: 'job', command: {id: 'stale'}});
  replacement.receive({type: 'job', command: {id: 'current'}});
  assert.equal((await pending).id, 'current');
  f.jobs.close();
});

test('retired sessions fail immediately and never reconnect', async () => {
  const f = harness();
  const socket = f.open();
  const rejected = assert.rejects(f.jobs.next(), error => error.code === 'SESSION_RETIRED');
  socket.receive({type: 'error', code: 'SESSION_RETIRED', message: 'Session changed'});
  await rejected;
  f.tick(120000);
  assert.equal(f.sockets.length, 1);
  assert.equal(f.timers.size, 0);
});

test('an initial connection that never opens has a bounded deadline', async () => {
  const f = harness();
  const rejected = assert.rejects(f.jobs.next(), /unavailable for 120 seconds/);
  f.tick(120000);
  await rejected;
  assert.equal(f.timers.size, 0);
});

test('a connection that never recovers cannot extend its deadline with retries', async () => {
  const f = harness();
  const socket = f.open();
  const rejected = assert.rejects(f.jobs.next(), /unavailable for 120 seconds/);
  socket.close(1006);
  f.tick(250);
  f.sockets[1].close(1006);
  f.tick(119750);
  await rejected;
  assert.equal(f.timers.size, 0);
});

test('result retries reuse the exact body and stop on a retired-session response', async () => {
  let clock = 0;
  const bodies = [];
  const body = '{"ok":true,"timings":{"execution_ms":42}}';
  const measured = await postResult(async value => {
    bodies.push(value);
    if (bodies.length === 1) throw new TypeError('Lost HTTP acknowledgment');
  }, body, {now: () => clock, sleep: async delay => {clock += delay;}});
  assert.deepEqual(bodies, [body, body]);
  assert.deepEqual(measured, {result_ack_ms: 250, result_attempts: 2});
  await assert.rejects(postResult(async () => {
    throw Object.assign(new Error('Retired'), {status: 400});
  }, body, {sleep: async () => assert.fail('Authentication failure must not retry')}), /Retired/);
});

test('result transport failures have a bounded retry budget', async () => {
  let clock = 0;
  await assert.rejects(postResult(async () => {
    throw new TypeError('Disconnected');
  }, '{}', {now: () => clock, timeoutMs: 1000, sleep: async delay => {clock += delay;}}), /Disconnected/);
  assert.ok(clock >= 1000 && clock < 2000);
});

test('disconnect during capture never launches a second execution', async () => {
  const f = harness();
  const first = f.open();
  let finishCapture;
  let executions = 0;
  const results = [];
  const worker = {
    execute() {
      executions++;
      return new Promise(resolve => {finishCapture = resolve;});
    },
    resources: () => ({})
  };
  const running = runJobs({jobs: f.jobs, worker, phase() {}, sleep: async () => {},
    sendResult: async (id, body) => {results.push({id, body});}});
  const rejected = assert.rejects(running, /connection closed/);
  first.receive({type: 'job', command: {id: '000001', kind: 'capture_images'}});
  await flush();
  first.close(1006);
  f.tick(250);
  const replacement = f.open();
  assert.equal(replacement.sent.filter(message => message.type === 'next').length, 0);
  finishCapture({timings: {execution_ms: 100}});
  await flush();
  assert.equal(executions, 1);
  assert.equal(results.length, 1);
  assert.equal(replacement.sent.filter(message => message.type === 'next').length, 1);
  f.jobs.close();
  await rejected;
});

test('duplicate completed command replays its result without recapturing', async () => {
  const f = harness();
  const socket = f.open();
  let executions = 0;
  const results = [];
  const worker = {
    async execute() {executions++; return {timings: {execution_ms: 10}};},
    resources: () => ({})
  };
  const running = runJobs({jobs: f.jobs, worker, phase() {}, sleep: async () => {},
    sendResult: async (id, body) => {results.push({id, body});}});
  const rejected = assert.rejects(running, /connection closed/);
  const job = {type: 'job', command: {id: '000001', kind: 'plan'}};
  socket.receive(job);
  await flush();
  socket.receive(job);
  await flush();
  assert.equal(executions, 1);
  assert.equal(results.length, 2);
  assert.equal(results[0].body, results[1].body);
  f.jobs.close();
  await rejected;
});

test('a new job is requested only after result acknowledgment', async () => {
  const f = harness();
  const socket = f.open();
  let acknowledge;
  const running = runJobs({jobs: f.jobs,
    worker: {async execute() {return {};}, resources: () => ({})},
    phase() {}, sleep: async () => {},
    sendResult: () => new Promise(resolve => {acknowledge = resolve;})});
  const rejected = assert.rejects(running, /connection closed/);
  socket.receive({type: 'job', command: {id: '000001'}});
  await flush();
  socket.receive({type: 'changed', revision: 2});
  assert.equal(socket.sent.filter(message => message.type === 'next').length, 1);
  acknowledge();
  await flush();
  assert.equal(socket.sent.filter(message => message.type === 'next').length, 2);
  f.jobs.close();
  await rejected;
});
