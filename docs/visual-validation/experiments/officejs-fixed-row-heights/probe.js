  async function probeFixedRowHeights(command, office) {
    const started = performance.now();
    return office(async context => {
      if (command.action === 'rebuild_save') {
        context.workbook.application.calculate(Excel.CalculationType.fullRebuild);
        await context.sync();
      }
      if (command.action === 'save' || command.action === 'rebuild_save') {
        context.workbook.save(Excel.SaveBehavior.save);
        await context.sync();
      }
      const ranges = command.sheets.map(name => {
        const sheet = context.workbook.worksheets.getItem(name);
        const row = sheet.getRange('A8');
        const next = sheet.getRange('A9');
        row.load('top,height,rowHidden');
        row.format.load('rowHeight');
        next.load('top');
        return {name, row, next};
      });
      await context.sync();
      return {
        operation: command.operation, action: command.action,
        elapsed_ms: performance.now() - started,
        document_url: Office.context.document.url,
        rows: ranges.map(({name, row, next}) => ({
          sheet: name, top: row.top, height: row.height,
          format_row_height: row.format.rowHeight,
          hidden: row.rowHidden, next_top: next.top,
        })),
      };
    });
  }
