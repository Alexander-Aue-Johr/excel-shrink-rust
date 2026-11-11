import type { Snapshot } from "./model";

export function worksheetOptions(
  snapshot: Snapshot,
  query: string,
  requestedPage: number,
  selected: string | null,
) {
  const names = new Map(snapshot.workbooks.map((book) => [book.id, book.name]));
  const search = query.trim().toLowerCase();
  const matches = snapshot.sheets.filter((sheet) =>
    `${sheet.workbookId} ${names.get(sheet.workbookId) ?? ""} ${sheet.name}`
      .toLowerCase()
      .includes(search),
  );
  const pageSize = 100;
  const pages = Math.max(1, Math.ceil(matches.length / pageSize));
  const page = Math.max(0, Math.min(pages - 1, requestedPage));
  const sheets = matches.slice(page * pageSize, (page + 1) * pageSize);
  const pinned = snapshot.sheets.find((sheet) => sheet.id === selected);
  if (pinned && !sheets.some((sheet) => sheet.id === pinned.id))
    sheets.unshift(pinned);
  return { sheets, page, pages, total: matches.length };
}
