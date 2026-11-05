/** Traverse region order first, then cross a workbook boundary without wrapping. */
export function adjacentDifference(regionCount, index, workbooks, id, direction) {
  const next = index + direction;
  if (next >= 0 && next < regionCount) return { regionIndex: next };
  const file = workbooks.findIndex(workbook => String(workbook.id) === String(id));
  if (file < 0) return null;
  const workbook = workbooks[file + direction];
  return workbook ? { workbook, selectLast: direction < 0 } : null;
}

/** Take one paginated snapshot of all different workbooks, independent of the sidebar filter. */
export async function loadDifferentWorkbooks(signal) {
  const workbooks = [];
  let offset = 0;
  let runId = null;
  do {
    const response = await fetch(`/api/workbooks?filter=differences&offset=${offset}&limit=100`,
      { cache: 'no-store', signal });
    const page = await response.json();
    if (!response.ok) throw new Error(page.error || `HTTP ${response.status}`);
    if (!page.available) throw new Error('Workbook index is not available yet.');
    if (runId != null && page.run_id !== runId) throw new Error('The run changed while loading its differences.');
    runId = page.run_id;
    workbooks.push(...(page.workbooks || []));
    if (page.next_offset == null) break;
    if (page.next_offset <= offset) throw new Error('Workbook pagination did not advance.');
    offset = page.next_offset;
  } while (true);
  return workbooks;
}
