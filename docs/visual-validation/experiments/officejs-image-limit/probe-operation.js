// Inserted into a temporary copy of resident-diagnosis.js by run.py.
// The production worker and its image limits are never modified.
/** Request one raw Excel image and return its dimensions plus inspectable PNG data. */
async function probeRangeImage(command, office) {
  return office(async context => {
    const firstRow = command.start_row ?? 1;
    const lastRow = firstRow + command.rows - 1;
    const image = context.workbook.worksheets.getItem(command.sheet_name)
      .getRange(`A${firstRow}:A${lastRow}`).getImage();
    await context.sync();

    const base64 = image.value;
    const header = atob(base64.slice(0, 44));
    const integer = offset => (header.charCodeAt(offset) * 16777216 +
      (header.charCodeAt(offset + 1) << 16) +
      (header.charCodeAt(offset + 2) << 8) + header.charCodeAt(offset + 3));
    const width = integer(16);
    const height = integer(20);
    return {
      operation: command.operation,
      start_row: firstRow,
      rows: command.rows,
      width,
      height,
      encoded_characters: base64.length,
      // Keep the returned image for inspecting silent truncation as well.
      image_base64: width * height <= 4000000 ? base64 : null
    };
  });
}

/** Capture and hash a bounded run of real Lysimeter rows without retaining PNGs. */
async function probeHashSweep(command, office) {
  const first = command.first_row;
  const last = command.last_row;
  const rowsPerTile = command.rows_per_tile;
  if (!Number.isInteger(first) || !Number.isInteger(last) ||
      !Number.isInteger(rowsPerTile) || first < 1 || last < first ||
      last > 1048576 || rowsPerTile < 1 || rowsPerTile > 1700 ||
      Math.ceil((last - first + 1) / rowsPerTile) > 20) {
    throw new Error('Invalid bounded hash-sweep request');
  }

  const tiles = [];
  for (let start = first; start <= last; start += rowsPerTile) {
    const end = Math.min(last, start + rowsPerTile - 1);
    const started = performance.now();
    const base64 = await office(async context => {
      const image = context.workbook.worksheets.getItem(command.sheet_name)
        .getRange(`J${start}:J${end}`).getImage();
      await context.sync();
      return image.value;
    });
    const header = atob(base64.slice(0, 44));
    const integer = offset => (header.charCodeAt(offset) * 16777216 +
      (header.charCodeAt(offset + 1) << 16) +
      (header.charCodeAt(offset + 2) << 8) + header.charCodeAt(offset + 3));
    const width = integer(16);
    const height = integer(20);
    if (width !== 65 || height !== 17 * (end - start + 1) + 1) {
      throw new Error(`Incomplete PNG for J${start}:J${end}: ${width} × ${height}`);
    }
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index++) {
      bytes[index] = binary.charCodeAt(index);
    }
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
    const sha256 = Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('');
    tiles.push({range: `J${start}:J${end}`, width, height, sha256,
      png_bytes: bytes.length, elapsed_ms: performance.now() - started});
  }
  return {operation: command.operation, tiles};
}
