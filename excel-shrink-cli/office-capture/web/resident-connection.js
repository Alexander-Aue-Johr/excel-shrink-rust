/* global WebSocket */
(function (root) {
  'use strict';

  const RECONNECT_LIMIT_MS = 120000;

  // A single waiter replaces the idle job poll. The durable command/result
  // records remain authoritative when a connection is lost and re-established.
  function createJobConnection({url, client, token, connection = () => {}}, environment = {}) {
    const createSocket = environment.createSocket || (address => new WebSocket(address));
    const now = environment.now || (() => performance.now());
    const schedule = environment.setTimeout || setTimeout;
    const cancel = environment.clearTimeout || clearTimeout;
    let socket;
    let waiter;
    let requested = false;
    let revision = -1;
    let requestedRevision = -1;
    let retryTimer;
    let deadlineTimer;
    let reconnectStarted;
    let attempts = 0;
    let terminalError;

    function clearTimers() {
      cancel(retryTimer);
      cancel(deadlineTimer);
      retryTimer = deadlineTimer = undefined;
    }

    function fail(error) {
      if (terminalError) return;
      terminalError = error;
      clearTimers();
      const previous = socket;
      socket = undefined;
      previous?.close();
      connection(false, error.message);
      if (waiter) {
        waiter.reject(error);
        waiter = undefined;
      }
    }

    function requestNext() {
      if (!waiter || requested || socket?.readyState !== 1 || terminalError) return;
      requested = true;
      requestedRevision = revision;
      socket.send(JSON.stringify({type: 'next'}));
    }

    function receive(message) {
      if (message.type === 'error') {
        const error = new Error(message.message || 'Resident event connection failed');
        error.code = message.code || 'EVENT_PROTOCOL_FAILED';
        fail(error);
        return;
      }
      if (!['changed', 'idle', 'job'].includes(message.type)) {
        throw new Error('Unknown resident event message');
      }
      // Only an authenticated server message ends the reconnect deadline.
      cancel(deadlineTimer);
      deadlineTimer = undefined;
      reconnectStarted = undefined;
      attempts = 0;
      connection(true, 'Resident event connection ready');
      if (message.type === 'changed') {
        if (!Number.isSafeInteger(message.revision) || message.revision < 0) {
          throw new Error('Invalid resident event revision');
        }
        if (message.revision > revision) {
          revision = message.revision;
          requestNext();
        }
        return;
      }
      if (!requested || !waiter) throw new Error('Unsolicited resident job response');
      requested = false;
      if (message.type === 'idle') {
        // A change can arrive while next is in flight. Reconcile it once;
        // otherwise remain asleep until another server event.
        const observedRevision = Number.isSafeInteger(message.revision)
          ? Math.max(requestedRevision, message.revision) : requestedRevision;
        if (revision > observedRevision) requestNext();
        return;
      }
      if (!message.command || typeof message.command.id !== 'string') {
        throw new Error('Resident job has no command identity');
      }
      const pending = waiter;
      waiter = undefined;
      pending.resolve(message.command);
    }

    function reconnect() {
      if (terminalError || retryTimer !== undefined) return;
      if (reconnectStarted === undefined) {
        reconnectStarted = now();
        deadlineTimer = schedule(() => fail(new Error('Resident event connection unavailable for 120 seconds')),
          RECONNECT_LIMIT_MS);
      }
      const remaining = RECONNECT_LIMIT_MS - (now() - reconnectStarted);
      if (remaining <= 0) {
        fail(new Error('Resident event connection unavailable for 120 seconds'));
        return;
      }
      const delay = Math.min(250 * 2 ** Math.min(attempts++, 5), 5000, remaining);
      retryTimer = schedule(() => {
        retryTimer = undefined;
        connect();
      }, delay);
    }

    function connect() {
      if (terminalError) return;
      if (reconnectStarted === undefined) {
        reconnectStarted = now();
        deadlineTimer = schedule(() => fail(new Error('Resident event connection unavailable for 120 seconds')),
          RECONNECT_LIMIT_MS);
      }
      let current;
      try {
        current = createSocket(url);
      } catch (_) {
        reconnect();
        return;
      }
      socket = current;
      requested = false;
      // Revisions belong to this host connection; reconnect always reconciles
      // durable state even when its initial revision equals the previous one.
      revision = requestedRevision = -1;
      current.onopen = () => {
        if (socket !== current || terminalError) return;
        current.send(JSON.stringify({role: 'worker', client, token}));
        requestNext();
      };
      current.onmessage = event => {
        if (socket !== current || terminalError) return;
        try {
          receive(JSON.parse(event.data));
        } catch (error) {
          fail(error);
        }
      };
      current.onerror = () => {
        // Browsers deliver close after an error; reconnect only once there.
        connection(false, 'Resident event connection interrupted');
      };
      current.onclose = event => {
        if (socket !== current || terminalError) return;
        socket = undefined;
        requested = false;
        if ([1008, 4001, 4003].includes(event.code)) {
          const error = new Error(event.reason || 'Resident session was retired');
          error.code = 'SESSION_RETIRED';
          fail(error);
          return;
        }
        connection(false, 'Reconnecting resident event connection');
        reconnect();
      };
    }

    connect();
    return {
      next() {
        if (terminalError) return Promise.reject(terminalError);
        if (waiter) return Promise.reject(new Error('Only one resident job may be requested at a time'));
        return new Promise((resolve, reject) => {
          waiter = {resolve, reject};
          requestNext();
        });
      },
      close() {
        fail(new Error('Resident event connection closed'));
      }
    };
  }

  function retryable(error) {
    return error.status === undefined || error.status === 408 || error.status === 429 || error.status >= 500;
  }

  async function postResult(send, body, {sleep, now = () => performance.now(), timeoutMs = RECONNECT_LIMIT_MS}) {
    const started = now();
    let attempts = 0;
    while (true) {
      attempts++;
      try {
        await send(body, Math.max(1, timeoutMs - (now() - started)));
        return {result_ack_ms: now() - started, result_attempts: attempts};
      } catch (error) {
        if (!retryable(error) || now() - started >= timeoutMs) throw error;
        await sleep(Math.min(250 * 2 ** Math.min(attempts - 1, 5), 5000,
          timeoutMs - (now() - started)));
        if (now() - started >= timeoutMs) throw error;
      }
    }
  }

  async function runJobs({jobs, worker, sendResult, sleep, phase, now = () => performance.now()}) {
    let completed;
    while (true) {
      phase('Waiting for comparison jobs');
      const waiting = now();
      const command = await jobs.next();
      let result;
      let body;
      if (completed?.id === command.id) {
        // A result can be committed while its HTTP acknowledgment is lost.
        // Replay precisely that result; never recapture the same command.
        result = completed.result;
        body = completed.body;
      } else {
        const jobWaitMs = now() - waiting;
        try {
          result = {ok: true, ...await worker.execute(command)};
        } catch (error) {
          result = {ok: false, error: {
            code: error?.code || 'CAPTURE_FAILED',
            message: error?.message || String(error)
          }};
        }
        result.timings = {...result.timings, job_wait_ms: jobWaitMs};
        result.pane_resources = worker.resources();
        body = JSON.stringify(result);
        completed = {id: command.id, result, body};
      }
      const acknowledgment = await postResult(
        (serialized, timeoutMs) => sendResult(command.id, serialized, timeoutMs), body, {sleep, now});
      worker.recordAcknowledgment?.(command, result.timings, acknowledgment);
      if (!result.ok) throw new Error(result.error.message);
    }
  }

  const api = {createJobConnection, postResult, runJobs};
  root.ResidentConnection = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
}(typeof window !== 'undefined' ? window : globalThis));
