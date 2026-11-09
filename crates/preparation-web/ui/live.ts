import type {
  DemoRun,
  SheetState,
  Snapshot,
  VisibleJob,
  WorkbookState,
} from "./lib/model";

export interface PackageSizes {
  file_bytes: number;
  worksheets: {
    part: string;
    compressed_bytes: number;
    uncompressed_bytes: number;
  }[];
}

export interface Job {
  id: string;
  name: string;
  worker: number | null;
  started_ms: number;
  ended_ms: number | null;
  status: string;
  before: PackageSizes | null;
  after: PackageSizes | null;
  source_bytes: number | null;
  error: string | null;
}

export interface Configuration {
  inputs: string[];
  output: string | null;
  workers: number;
  recursive: boolean;
  reserve_mib: number | null;
  source_manifest: string | null;
  reuse_from: string | null;
}

interface NativeEvent {
  event_type: string;
  elapsed_millis: number;
  workbook_id: string | null;
  worker_index: number | null;
  payload: {
    name?: string;
    package?: PackageSizes;
    file_bytes?: number;
    archive_sizes?: {
      before_optimization?: PackageSizes;
      after_optimization?: PackageSizes;
      converted?: PackageSizes;
    };
    error?: string;
  };
}

export interface Update {
  run_id: string;
  operation: "xlsx-convert" | "check-performance";
  status: string;
  error: string | null;
  defaults: Configuration;
  elapsed_ms: number;
  next: number;
  reset: boolean;
  jobs: Job[] | null;
  counts: {
    total: number;
    prepared: number;
    failed: number;
    reused: number;
    before_bytes: number;
    after_bytes: number;
  };
  events: { cursor: number; event: NativeEvent }[];
  history_limit: number;
}

export function applyUpdate(previous: Job[], update: Update): Job[] {
  if (update.reset) return update.jobs ?? [];
  const jobs = new Map(previous.map((job) => [job.id, { ...job }]));
  for (const { event } of update.events) {
    const id = event.workbook_id;
    if (!id) continue;
    if (event.event_type === "job.started") {
      jobs.set(id, {
        id,
        name: event.payload.name ?? id,
        worker: event.worker_index,
        started_ms: event.elapsed_millis,
        ended_ms: null,
        status: "running",
        before: null,
        after: null,
        source_bytes: null,
        error: null,
      });
    }
    const job = jobs.get(id);
    if (!job) continue;
    if (event.event_type === "workbook.metadata") {
      job.before = event.payload.package ?? null;
      job.source_bytes = event.payload.file_bytes ?? null;
    }
    if (
      event.event_type === "job.completed" ||
      event.event_type === "job.failed"
    ) {
      job.ended_ms = event.elapsed_millis;
      job.status = event.event_type === "job.completed" ? "prepared" : "failed";
      job.before =
        event.payload.archive_sizes?.before_optimization ?? job.before;
      job.after =
        event.payload.archive_sizes?.after_optimization ??
        event.payload.archive_sizes?.converted ??
        null;
      job.error = event.payload.error ?? null;
    }
  }
  const retained = [...jobs.values()];
  while (retained.length > update.history_limit) {
    const position = retained.findIndex((job) => job.ended_ms !== null);
    if (position < 0) break;
    retained.splice(position, 1);
  }
  return retained;
}

/** Adapt measured workbook jobs to the demo's renderer contract without inventing sheet jobs. */
export function scenes(
  update: Update,
  jobs: Job[],
): { snapshot: Snapshot; run: DemoRun } {
  const time = update.elapsed_ms / 1000;
  const books: WorkbookState[] = jobs.map((job, index) => {
    const after = new Map(
      job.after?.worksheets.map((sheet) => [sheet.part, sheet]) ?? [],
    );
    const sheets: SheetState[] = (job.before?.worksheets ?? []).map(
      (sheet, sheetIndex) => ({
        id: `${job.id}:${sheet.part}`,
        workbookId: job.id,
        index: sheetIndex + 1,
        name: sheet.part,
        originalBytes: sheet.uncompressed_bytes,
        cleanedBytes: after.get(sheet.part)?.uncompressed_bytes ?? null,
        chunkCount: 1,
        hue: (sheetIndex * 43) % 360,
        done: job.status === "prepared",
      }),
    );
    return {
      id: job.id,
      index,
      name: job.name,
      known: job.before !== null,
      status: job.ended_ms === null ? "processing" : "complete",
      originalBytes: job.before?.file_bytes ?? job.source_bytes,
      cleanedBytes: job.after?.file_bytes ?? null,
      sheets,
      startAt: job.started_ms / 1000,
      completedAt: job.ended_ms === null ? null : job.ended_ms / 1000,
      progress: job.ended_ms === null ? 0 : 1,
    };
  });
  const visible: VisibleJob[] = jobs.map((job, index) => ({
    id: job.id,
    workbookId: job.id,
    sheetId: job.id,
    workerIndex: job.worker ?? 0,
    startAt: job.started_ms / 1000,
    endAt: (job.ended_ms ?? update.elapsed_ms) / 1000,
    visibleEnd: Math.min(time, (job.ended_ms ?? update.elapsed_ms) / 1000),
    chunkIndex: 0,
    chunkCount: 1,
    done: job.ended_ms !== null,
    workbookIndex: index,
    workbookName: job.name,
    sheet: {
      id: job.id,
      workbookId: job.id,
      index: 1,
      name: job.name,
      originalBytes: job.before?.file_bytes ?? job.source_bytes ?? 0,
      cleanedBytes: job.after?.file_bytes ?? null,
      chunkCount: 1,
      hue: job.status === "failed" || job.status === "interrupted" ? 0 : 155,
      done: job.ended_ms !== null,
    },
  }));
  const sheets = books.flatMap((book) => book.sheets);
  const snapshot: Snapshot = {
    time,
    workbooks: books,
    sheets,
    jobs: visible,
    events: [],
    activeJobs: visible.filter((job) => !job.done),
    completedBooks: jobs.filter((job) => job.status === "prepared").length,
    knownBooks: books.filter((book) => book.known).length,
    processingBooks: visible.filter((job) => !job.done).length,
    completedSheets: sheets.filter((sheet) => sheet.done).length,
    beforeBytes: update.counts.before_bytes,
    afterBytes: update.counts.after_bytes,
    savedBytes: update.counts.before_bytes - update.counts.after_bytes,
    reduction: update.counts.before_bytes
      ? 1 - update.counts.after_bytes / update.counts.before_bytes
      : 0,
  };
  const workerCount = Math.max(
    update.defaults.workers,
    ...jobs.map((job) => (job.worker ?? 0) + 1),
  );
  const run: DemoRun = {
    id: update.run_id,
    size: "small",
    workerCount,
    workbooks: [],
    jobs: [],
    events: [],
    duration: time,
    sheetCount: sheets.length,
  };
  return { snapshot, run };
}
