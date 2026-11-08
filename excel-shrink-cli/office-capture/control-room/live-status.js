/* Shared bounded live connection for the summary and capture-job panel. */
export function applyLivePacket(previous, message) {
  if (!Number.isSafeInteger(message.sequence) || message.sequence < 1) throw new Error('Invalid live revision');
  if (message.snapshot) return { sequence: message.sequence, value: message.snapshot };
  if (!previous || message.sequence !== previous.sequence + 1) throw new Error('Live revision gap');
  const value = { ...previous.value, comparisons: { ...previous.value.comparisons } };
  for (const operation of message.patch || []) {
    const parts = operation.path.split('/').slice(1);
    if (parts.length === 1 && ['queue', 'retired_run_ids', 'diagnostics'].includes(parts[0])
      && operation.op === 'replace') {
      value[parts[0]] = operation.value;
    } else if (parts.length === 2 && parts[0] === 'comparisons'
      && /^[0-9a-f-]{36}$/i.test(parts[1])) {
      if (operation.op === 'remove') delete value.comparisons[parts[1]];
      else if (['add', 'replace'].includes(operation.op)) value.comparisons[parts[1]] = operation.value;
      else throw new Error('Unsupported live operation');
    } else throw new Error('Unsupported live path');
  }
  return { sequence: message.sequence, value };
}

let socket = null;
let packet = null;
let retryAt = 0;
let receivedAt = 0;

/** A write must not be followed by a cached pre-write control snapshot. */
export function invalidateLiveStatus() {
  packet = null;
  const previous = socket;
  socket = null;
  retryAt = 0;
  previous?.close();
}

/** Reconnect with a fresh baseline. Patches never cross connection boundaries. */
export function liveStatus() {
  if (typeof WebSocket === 'undefined') return null;
  if (packet && Date.now() - receivedAt > 15000) {
    packet = null;
    socket?.close();
  }
  if (!socket && Date.now() >= retryAt && !document.hidden) {
    packet = null;
    const connection = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/api/live`);
    socket = connection;
    connection.addEventListener('message', event => {
      try {
        packet = applyLivePacket(packet, JSON.parse(event.data));
        receivedAt = Date.now();
      }
      catch { packet = null; connection.close(); }
    });
    connection.addEventListener('close', () => {
      if (socket !== connection) return;
      socket = null;
      packet = null;
      retryAt = Date.now() + 30000;
    });
  }
  return socket?.readyState === WebSocket.OPEN ? packet?.value || null : null;
}
