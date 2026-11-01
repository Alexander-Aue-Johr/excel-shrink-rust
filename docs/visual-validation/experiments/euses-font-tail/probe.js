async function inspectFontTail(command, office) {
  return office(async context => {
    const sheet = context.workbook.worksheets.getItem(command.sheet || 'Sheet1');
    const captures = command.ranges.map(address => {
      const range = sheet.getRange(address);
      range.load('top,height,width');
      return {address, range, image: range.getImage()};
    });
    const addresses = command.rows || ['A52', 'A53', 'A54', 'A1000', 'A65445', 'A65446'];
    const rows = addresses.map(address => {
      const range = sheet.getRange(address);
      range.load('top,height');
      range.format.load('rowHeight');
      range.format.font.load('name,size');
      return {address, range};
    });
    const tail = sheet.getRange(command.tail || 'A53:A65445');
    tail.load('top,height');
    tail.format.load('rowHeight');
    await context.sync();
    return {
      operation: command.operation,
      captures: captures.map(({address, range, image}) => ({
        address, top: range.top, height: range.height, width: range.width, image: image.value
      })),
      rows: rows.map(({address, range}) => ({
        address, top: range.top, height: range.height, row_height: range.format.rowHeight,
        font_name: range.format.font.name, font_size: range.format.font.size
      })),
      whole_tail: {top: tail.top, height: tail.height, uniform_row_height: tail.format.rowHeight}
    };
  });
}
