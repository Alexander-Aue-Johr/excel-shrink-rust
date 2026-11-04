import { acceptComparison, captureProgress, comparisonFinished, heartbeatRow, iconActivity, phaseComplete, processCards } from '../targeted-model.js';

export function runTests() {
  let passed = 0;
  const assert = (condition, message) => { if (!condition) throw new Error(message); passed++; };
  assert(phaseComplete([{ steps: { open: 'done' } }, { steps: { open: 'skipped' } }], 'open'), 'Skipped lanes do not block completed phases.');
  assert(!phaseComplete([{ steps: { open: 'done' } }, { steps: { open: 'running' } }], 'open'), 'Running lanes prevent completion.');
  assert(!phaseComplete([{ steps: { open: 'skipped' } }], 'open'), 'An entirely skipped phase is not certified complete.');
  assert(!phaseComplete([{ steps: { open: 'done' } }, {}], 'open'), 'Missing status must not imply completion.');
  const ended = { run_id: 'one', status: 'completed_with_limitations', finished_at: 2000,
    lanes: [{ id: 'O', pid: 12, lifecycle: 'closed', process_alive: false }, { id: 'CP', lifecycle: 'unavailable' }] };
  assert(comparisonFinished(ended), 'Unavailable CP does not pretend to be a live process.');
  assert(!comparisonFinished({ ...ended, lanes: [{ pid: 12, lifecycle: 'open', process_alive: true }] }),
    'A completed report cannot move a still-open Excel process into completed history.');
  assert(!comparisonFinished({ ...ended, finished_at: null }), 'An unconfirmed completion stays active.');
  const opening = iconActivity({ lifecycle: 'opening' }, 'excel', 1000);
  assert(opening.pulse && opening.level === 1, 'Excel pulses specifically while opening.');
  const open = iconActivity({ lifecycle: 'open' }, 'excel', 1000);
  assert(!open.pulse && open.level === 1, 'Open Excel stays steadily illuminated.');
  const closed = { lifecycle: 'closed', closed_at: 1000 };
  assert(iconActivity(closed, 'excel', 1000).level === 1, 'Confirmed exit begins a smooth afterglow.');
  const faded = iconActivity(closed, 'excel', 7500).level;
  assert(faded > 0.3 && faded < 0.4, 'Afterglow depends on real elapsed event time.');
  assert(iconActivity({ ...closed }, 'excel', 14000).level < faded, 'Repeated snapshots cannot restart the fade.');
  const fastXml = { xml_started_at: 1000, xml_finished_at: 1001 };
  assert(iconActivity(fastXml, 'xml', 2000).level > 0.5, 'A one-millisecond XML operation remains visible after a poll.');
  assert(iconActivity({}, 'xml', 2000).level === 0, 'Missing XML events never create activity.');
  assert(iconActivity({ xml_started_at: 3000 }, 'xml', 2000).level === 0, 'Future timestamps do not invent current XML activity.');
  assert(!captureProgress({ captures_done: 0 }).known, 'Unknown queue size remains indeterminate.');
  assert(!captureProgress({ captures_done: 6, captures_total: 5 }).known, 'Invalid counts never become a completed bar.');
  assert(captureProgress({ captures_done: 2, captures_total: 8 }).remaining === 6, 'A growing candidate queue preserves completed work.');
  assert(captureProgress({ captures_done: 0, captures_total: 0 }).known, 'Zero-candidate selection is explicit.');
  assert(captureProgress({ captures_done: 0, captures_total: 0, capture_selection_complete: true })
    .text === 'No capture candidates found', 'Completed empty selection has an explicit reason.');
  assert(captureProgress({ lifecycle: 'skipped', decision_note: 'Default unchanged — identical input bytes' })
    .text.includes('identical input'), 'Skipped states retain the proof explanation instead of a fake progress counter.');
  assert(captureProgress({ captures_done: 3, captures_total: 3, skipped_zero_area_regions: 2 })
    .text.includes('2 skipped: no visible area'), 'Non-rendering observations are not presented as captured images.');
  const current = { run_id: 'one', revision: 5, lanes: [] };
  assert(acceptComparison(current, { ...current, revision: 4 }) === current, 'Late replies cannot reverse the lifecycle.');
  const row = heartbeatRow(ended, { id: 'O', pid: 12, process_started_at: 50, process_alive: true });
  assert(row.heartbeatCount === undefined && row.heartbeat == null, 'A PID without a real probe never becomes a heartbeat.');
  assert(row.heartbeatSource.includes('Office.js'), 'The heart trace identifies the actual add-in source.');
  const probed = { id: 'O', pid: 12, process_alive: true, process_heartbeat_at: 5000, process_heartbeat_count: 50 };
  assert(heartbeatRow(ended, probed).heartbeat == null, 'Native PID samples never generate an Office.js beat.');
  const received = heartbeatRow(ended, { ...probed, addin_heartbeat_at: 4200, addin_heartbeat_count: 7 });
  assert(received.heartbeat === 4200 && received.heartbeatCount === 7, 'Only received add-in timestamps and counts reach the heart trace.');
  assert(iconActivity(probed, 'process').level === 1 && !iconActivity(probed, 'process').pulse, 'A live process lights its stationary cyan chip.');
  assert(iconActivity({ ...probed, lifecycle: 'failed' }, 'excel').level === 0, 'A living process alone cannot light the workbook LED.');
  const states = [
    { id: 'O', pid: 10, lifecycle: 'open' },
    { id: 'SD', pid: 20, lifecycle: 'closed' },
    { id: 'SDR', pid: 20, lifecycle: 'open' },
    { id: 'OR', lifecycle: 'not_started' },
    { id: 'CP', lifecycle: 'unavailable' },
    { id: 'CPR', lifecycle: 'skipped' },
  ];
  const cards = processCards(states);
  assert(cards.length === 2, 'Two resident processes produce two cards.');
  assert(cards[1].id === 'SD' && cards[1].pid === 20 && cards[1].label.includes('After rebuild'),
    'Default retains its card identity when the same process advances to rebuild.');
  assert(processCards([{ id: 'SD', lifecycle: 'opening' }])[0].label.includes('Before rebuild'),
    'Pending Default remains visible before its PID is known.');
  assert(processCards([...states, { id: 'CP', pid: 30, lifecycle: 'open' }]).length === 3,
    'An actual optional Check Performance process remains visible.');
  const rebuiltOriginal = processCards([
    { id: 'O', pid: 10, lifecycle: 'open' },
    { id: 'OR', pid: 10, lifecycle: 'open' },
    { id: 'SD', pid: 20, lifecycle: 'open' },
    { id: 'SDR', pid: 20, lifecycle: 'open' },
    { id: 'CP', pid: 30, lifecycle: 'open' },
    { id: 'CPR', pid: 30, lifecycle: 'open' },
  ]);
  assert(rebuiltOriginal.length === 3 && rebuiltOriginal.every(card => card.label.includes('After rebuild')),
    'Original, Default and optional CP each keep one card after their in-place rebuild.');
  return passed;
}

export const result = runTests();
