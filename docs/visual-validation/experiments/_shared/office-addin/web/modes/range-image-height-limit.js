/* global VVRenderDiagnostics */
'use strict';
(async function () {
  const D = VVRenderDiagnostics;
  let session;
  try {
    session = await D.open('range-image-height-limit');
    if (!session) return;
    const workbook = 'krankenhausverzeichnis-3500100227005.xlsx';
    if (session.workbook.toLowerCase() !== workbook.toLowerCase()) throw new Error('Wrong diagnostic workbook: ' + session.workbook);
    const sheet = ' KHV_2022';
    session.analysis.target = { sheet, fixed_columns: 'A:FX', max_row: 2791, original_sheet_not_modified: true };
    const boundary = await D.plateauSearch(session, {
      sheet, axis: 'y', maxIndex: 2791, baseline: [1, 40, 100],
      address: (n) => `A1:FX${n}`, controlAddress: 'A1:FX40', neighborhoodRadius: 5
    });
    session.analysis.height_plateau = boundary;
    // Width controls are labelled separately; they never change the fixed-width search result.
    const controls = [];
    if (!session.halted && boundary.conclusion === 'plateau-boundary-locally-confirmed') {
      for (const column of ['H', 'AZ']) {
        for (const row of [boundary.first_plateau_index - 1, boundary.first_plateau_index + 1, 2791]) {
          controls.push(await session.probe({ sheet, address: `A1:${column}${row}`,
            role: 'width-control-after-fixed-FX-search', control_last_column: column, last_row: row }));
        }
      }
    }
    session.analysis.width_controls = controls;
    session.analysis.scope_note = 'Row-resolution saturation boundary for this workbook/Excel run, not a universal PNG limit. Clipping vs scaling is tested separately with synthetic markers.';
    await session.complete((session.halted || boundary.conclusion === 'inconclusive' || controls.some((p) => !p.success)) ? 'inconclusive' : 'complete');
  } catch (e) { await D.finishFailure(session, e); }
}());
