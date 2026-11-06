/** Read-only diagnostic reuse with explicit entry and JSON-size budgets.
 * Eviction discards a display convenience; durable report evidence stays intact. */
export class DiagnosticCache {
  constructor({ maxEntries = 128, maxBytes = 4 * 1024 * 1024 } = {}) {
    this.maxEntries = maxEntries;
    this.maxBytes = maxBytes;
    this.entries = new Map();
    this.bytes = 0;
  }

  get(key) {
    const entry = this.entries.get(key);
    if (!entry) return undefined;
    this.entries.delete(key);
    this.entries.set(key, entry);
    return entry.value;
  }

  set(key, value) {
    this.delete(key);
    // Strings are held as UTF-16 in JavaScript; container/allocator overhead is
    // additional. This budget is deliberately not advertised as measured RSS.
    const bytes = JSON.stringify(value).length * 2;
    if (bytes > this.maxBytes || this.maxEntries < 1) return;
    while (this.entries.size >= this.maxEntries || this.bytes + bytes > this.maxBytes) {
      this.delete(this.entries.keys().next().value);
    }
    this.entries.set(key, { value, bytes });
    this.bytes += bytes;
  }

  delete(key) {
    const entry = this.entries.get(key);
    if (!entry) return;
    this.bytes -= entry.bytes;
    this.entries.delete(key);
  }
}
