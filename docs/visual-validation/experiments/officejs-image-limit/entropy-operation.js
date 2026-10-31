// Inserted into a disposable resident copy by entropy_probe.py.
/** Measure one raw Office.js image, hashing its PNG bytes when requested. */
async function probeEntropyImage(command, office) {
  const started = performance.now();
  const base64 = await office(async context => {
    const image = context.workbook.worksheets.getItem(command.sheet_name)
      .getRangeByIndexes(0, 0, command.rows, command.columns).getImage();
    await context.sync();
    return image.value;
  });
  const captureMs = performance.now() - started;
  const header = atob(base64.slice(0, 44));
  const signature = [137, 80, 78, 71, 13, 10, 26, 10];
  if (!signature.every((byte, index) => header.charCodeAt(index) === byte) ||
      header.slice(12, 16) !== 'IHDR') {
    throw new Error('Excel returned an invalid PNG');
  }
  const readInteger = offset => header.charCodeAt(offset) * 16777216 +
    (header.charCodeAt(offset + 1) << 16) +
    (header.charCodeAt(offset + 2) << 8) + header.charCodeAt(offset + 3);
  const result = {
    operation: command.operation,
    columns: command.columns,
    rows: command.rows,
    width: readInteger(16),
    height: readInteger(20),
    capture_ms: captureMs,
    png_bytes: Math.floor(base64.length * 3 / 4) -
      (base64.endsWith('==') ? 2 : base64.endsWith('=') ? 1 : 0)
  };
  if (command.hash) {
    const decodedAt = performance.now();
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index++) {
      bytes[index] = binary.charCodeAt(index);
    }
    result.decode_ms = performance.now() - decodedAt;
    const hashAt = performance.now();
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
    result.hash_ms = performance.now() - hashAt;
    result.sha256 = Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('');
  }
  if (command.save_png) result.image_base64 = base64;
  return result;
}
