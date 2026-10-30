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
