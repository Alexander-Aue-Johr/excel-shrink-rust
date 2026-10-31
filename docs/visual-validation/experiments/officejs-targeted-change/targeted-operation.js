// Inserted into a disposable resident copy by run.py.
async function probeTargetedImage(command, office) {
  const started = performance.now();
  const base64 = await office(async context => {
    const image = context.workbook.worksheets.getItem(command.sheet_name)
      .getRange(command.range).getImage();
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
  const dimension = offset => header.charCodeAt(offset) * 16777216 +
    (header.charCodeAt(offset + 1) << 16) +
    (header.charCodeAt(offset + 2) << 8) + header.charCodeAt(offset + 3);
  const decodedAt = performance.now();
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index++) {
    bytes[index] = binary.charCodeAt(index);
  }
  const decodeMs = performance.now() - decodedAt;
  const hashAt = performance.now();
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
  const hashMs = performance.now() - hashAt;
  return {
    operation: command.operation,
    sheet_name: command.sheet_name,
    range: command.range,
    width: dimension(16),
    height: dimension(20),
    png_bytes: bytes.length,
    capture_ms: captureMs,
    decode_ms: decodeMs,
    hash_ms: hashMs,
    sha256: Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join(''),
    image_base64: base64
  };
}

async function probeObjectBounds(command, office) {
  const started = performance.now();
  const objects = await office(async context => {
    const sheet = context.workbook.worksheets.getItem(command.sheet_name);
    const charts = sheet.charts;
    const shapes = sheet.shapes;
    charts.load('items/name,items/left,items/top,items/width,items/height');
    shapes.load('items/name,items/left,items/top,items/width,items/height,items/rotation,items/visible');
    await context.sync();
    return {
      charts: charts.items.map(chart => ({
        name: chart.name, left: chart.left, top: chart.top,
        width: chart.width, height: chart.height
      })),
      shapes: shapes.items.map(shape => ({
        name: shape.name, left: shape.left, top: shape.top,
        width: shape.width, height: shape.height,
        rotation: shape.rotation, visible: shape.visible
      }))
    };
  });
  return {operation: command.operation, elapsed_ms: performance.now() - started, ...objects};
}
