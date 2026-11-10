import type {
  DemoRun,
  SheetState,
  Snapshot,
  VisibleJob,
  WorkbookState,
} from "./lib/model";

export interface PackageSizes {
  file_bytes: number;
  uncompressed_payload_bytes?: number;
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
  sheet_names?: Record<string, string> | null;
  sheet_order?: string[] | null;
}

export interface WorkTask {
  id: number;
  workbook_id: string;
  sheet: string;
  worker: number | null;
  phase: "initial" | "finalize" | "chunk";
  chunk_index: number | null;
  chunk_count: number;
  range: string | null;
  started_ms: number;
  ended_ms: number | null;
}

export interface Configuration {
  inputs: string[];
  output: string | null;
  workers: number;
  recursive: boolean;
  reserve_mib: number | null;
  source_manifest: string | null;
  reuse_from: string | null;
  max_active_workbooks?: number | null;
  parallel_range_chunk_size?: number | null;
}

export interface NativeEvent {
  event_type: string;
  elapsed_millis: number;
  elapsed_micros?: number;
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
    sheet_names?: Record<string, string>;
    sheet_order?: string[];
    task_id?: number;
    sheet?: string;
    phase?: WorkTask["phase"];
    chunk_index?: number;
    chunk_count?: number;
    range?: string;
  };
}

export interface Update {
  run_id: string;
  operation: "xlsx-convert" | "check-performance" | "sheet-shrink";
  status: string;
  error: string | null;
  defaults: Configuration;
  elapsed_ms: number;
  next: number;
  reset: boolean;
  jobs: Job[] | null;
  tasks?: WorkTask[] | null;
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
      job.sheet_names = event.payload.sheet_names ?? null;
      job.sheet_order = event.payload.sheet_order ?? null;
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

export function applyTasks(
  previous: WorkTask[],
  update: Update,
  limit = 4096,
): WorkTask[] {
  if (update.reset) return update.tasks ?? [];
  const tasks = new Map(previous.map((task) => [task.id, { ...task }]));
  for (const { event } of update.events) {
    const id = event.payload?.task_id;
    if (id === undefined) continue;
    const time =
      event.elapsed_micros === undefined
        ? event.elapsed_millis
        : event.elapsed_micros / 1000;
    if (
      event.event_type === "work.started" &&
      event.workbook_id &&
      event.payload.sheet &&
      event.payload.phase
    ) {
      tasks.set(id, {
        id,
        workbook_id: event.workbook_id,
        sheet: event.payload.sheet,
        phase: event.payload.phase,
        worker: event.worker_index,
        chunk_index: event.payload.chunk_index ?? null,
        chunk_count: event.payload.chunk_count ?? 1,
        range: event.payload.range ?? null,
        started_ms: time,
        ended_ms: null,
      });
    }
    if (event.event_type === "work.finished") {
      const task = tasks.get(id);
      if (task) task.ended_ms = time;
    }
  }
  const retained = [...tasks.values()];
  while (retained.length > limit) {
    const position = retained.findIndex((task) => task.ended_ms !== null);
    if (position < 0) break;
    retained.splice(position, 1);
  }
  return retained;
}

/** Adapt measured work to the demo's renderer contract. */
export function scenes(
  update: Update,
  jobs: Job[],
  tasks: WorkTask[] = [],
): { snapshot: Snapshot; run: DemoRun } {
  const time = update.elapsed_ms / 1000;
  const chunks = new Map<string, number>();
  for (const task of tasks) {
    const key = `${task.workbook_id}:${task.sheet}`;
    chunks.set(key, Math.max(chunks.get(key) ?? 1, task.chunk_count));
  }
  const books: WorkbookState[] = jobs.map((job, index) => {
    const sourceIndex = /^workbook-(\d+)$/.exec(job.id);
    const workbookHue = (155 + Number(sourceIndex?.[1] ?? index) * 67) % 360;
    const after = new Map(
      job.after?.worksheets.map((sheet) => [sheet.part, sheet]) ?? [],
    );
    const sheets: SheetState[] = (job.before?.worksheets ?? []).map(
      (sheet, sheetIndex) => {
        const position = job.sheet_order?.indexOf(sheet.part) ?? -1;
        const xmlIndex = /sheet(\d+)\.xml$/.exec(sheet.part)?.[1];
        const index =
          position >= 0 ? position + 1 : Number(xmlIndex ?? sheetIndex + 1);
        let indexLabel = xmlIndex ? `XML${xmlIndex}` : "Sheet";
        if (position >= 0) indexLabel = `S${index}`;
        return {
          id: `${job.id}:${sheet.part}`,
          workbookId: job.id,
          index,
          indexLabel,
          name: job.sheet_names?.[sheet.part] ?? sheet.part,
          originalBytes: sheet.uncompressed_bytes,
          cleanedBytes: after.get(sheet.part)?.uncompressed_bytes ?? null,
          chunkCount: chunks.get(`${job.id}:${sheet.part}`) ?? 1,
          hue:
            (workbookHue + Number(xmlIndex ?? sheetIndex + 1) * 137.508) % 360,
          done: job.status === "prepared",
        };
      },
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
  const workbookJobs: VisibleJob[] = jobs.map((job, index) => ({
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
  const sheetsById = new Map(sheets.map((sheet) => [sheet.id, sheet]));
  const booksById = new Map(books.map((book) => [book.id, book]));
  const visible: VisibleJob[] =
    update.operation === "sheet-shrink"
      ? tasks.flatMap((task) => {
          const book = booksById.get(task.workbook_id);
          const sheet = sheetsById.get(`${task.workbook_id}:${task.sheet}`);
          if (!book || !sheet || task.worker === null) return [];
          return [
            {
              id: `task-${task.id}`,
              workbookId: task.workbook_id,
              sheetId: sheet.id,
              workerIndex: task.worker,
              workbookIndex: book.index,
              workbookName: book.name,
              startAt: task.started_ms / 1000,
              endAt: (task.ended_ms ?? update.elapsed_ms) / 1000,
              visibleEnd: (task.ended_ms ?? update.elapsed_ms) / 1000,
              chunkIndex: task.chunk_index ?? 0,
              chunkCount: task.chunk_count,
              phase: task.phase,
              done: task.ended_ms !== null,
              sheet,
            },
          ];
        })
      : workbookJobs;
  const snapshot: Snapshot = {
    time,
    workbooks: books,
    sheets,
    jobs: visible,
    events: [],
    activeJobs: visible.filter((job) => !job.done),
    completedBooks: jobs.filter((job) => job.status === "prepared").length,
    knownBooks: books.filter((book) => book.known).length,
    processingBooks: jobs.filter((job) => job.ended_ms === null).length,
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

/** Atlas frame and sheet columns share the same uncompressed ZIP byte scale. */
export function atlasSnapshot(snapshot: Snapshot, jobs: Job[]): Snapshot {
  const sources = new Map(jobs.map((job) => [job.id, job]));
  return {
    ...snapshot,
    workbooks: snapshot.workbooks.map((book) => {
      const source = sources.get(book.id);
      return {
        ...book,
        originalBytes: source?.before
          ? (source.before.uncompressed_payload_bytes ??
            book.sheets.reduce(
              (total, sheet) => total + sheet.originalBytes,
              0,
            ))
          : null,
        cleanedBytes: source?.after
          ? (source.after.uncompressed_payload_bytes ??
            book.sheets.reduce(
              (total, sheet) => total + (sheet.cleanedBytes ?? 0),
              0,
            ))
          : null,
      };
    }),
  };
}
