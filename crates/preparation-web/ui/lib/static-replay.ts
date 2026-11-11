import type { NativeEvent, Update } from "../live";

/** Load a frozen same-origin recording without contacting a validation API. */
export async function loadStaticReplay(
  path: string,
  signal: AbortSignal,
  request = fetch,
) {
  const manifestResponse = await request(path, { signal, redirect: "error" });
  if (!manifestResponse.ok)
    throw new Error("Static replay manifest unavailable");
  const manifest = await manifestResponse.json();
  if (manifest.schema !== 1 || manifest.events_file !== "events.jsonl.gz")
    throw new Error("Unsupported static replay manifest");
  if (
    !Number.isFinite(manifest.source?.elapsed_ms) ||
    manifest.source.elapsed_ms < 0 ||
    manifest.source.operation !== "sheet-shrink" ||
    !Number.isInteger(manifest.event_count) ||
    manifest.event_count < 1 ||
    manifest.event_count > 1000000 ||
    !Number.isInteger(manifest.events_bytes) ||
    manifest.events_bytes < 1 ||
    manifest.events_bytes > 128 * 1024 * 1024
  )
    throw new Error("Unsupported static replay size or source");
  const url = new URL(manifest.events_file, manifestResponse.url);
  const response = await request(url, { signal, redirect: "error" });
  if (!response.ok) throw new Error("Static replay events unavailable");
  const compressed = await readBounded(response.body!, 128 * 1024 * 1024);
  if (compressed.byteLength !== manifest.events_bytes)
    throw new Error("Static replay event size mismatch");
  const digest = await crypto.subtle.digest("SHA-256", compressed);
  const actual = Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
  if (actual !== manifest.events_sha256)
    throw new Error("Static replay event hash mismatch");
  const stream = new Blob([compressed])
    .stream()
    .pipeThrough(new DecompressionStream("gzip"));
  const text = new TextDecoder().decode(
    await readBounded(stream, 512 * 1024 * 1024),
  );
  const events: NativeEvent[] = text
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  if (events.length !== manifest.event_count)
    throw new Error("Static replay event count mismatch");
  let previous = -1;
  for (let index = 0; index < events.length; index += 1) {
    const event = events[index] as NativeEvent & { sequence: number };
    const time = event.elapsed_micros ?? event.elapsed_millis * 1000;
    if (event.sequence !== index || !Number.isFinite(time) || time < previous)
      throw new Error("Static replay sequence or clock mismatch");
    previous = time;
  }
  return { source: manifest.source as Update, events };
}

async function readBounded(stream: ReadableStream<Uint8Array>, limit: number) {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let bytes = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.byteLength;
      if (bytes > limit)
        throw new Error("Static replay exceeds the browser load limit");
      chunks.push(value);
    }
  } catch (error) {
    await reader.cancel();
    throw error;
  } finally {
    reader.releaseLock();
  }
  const output = new Uint8Array(bytes);
  let offset = 0;
  for (const chunk of chunks) {
    output.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return output.buffer;
}
