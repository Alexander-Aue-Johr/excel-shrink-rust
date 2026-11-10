import { build } from "esbuild";
import { test } from "node:test";
import assert from "node:assert/strict";

const result = await build({
  entryPoints: ["live.ts"],
  bundle: true,
  write: false,
  platform: "node",
  format: "esm",
});
const { applyUpdate, applyTasks, scenes, atlasSnapshot } = await import(
  `data:text/javascript;base64,${Buffer.from(result.outputFiles[0].contents).toString("base64")}`
);

const packageSize = (fileBytes, sheetBytes) => ({
  file_bytes: fileBytes,
  worksheets: [
    {
      part: "xl/worksheets/sheet1.xml",
      uncompressed_bytes: sheetBytes,
      compressed_bytes: 5,
    },
  ],
});
const base = {
  run_id: "one",
  operation: "check-performance",
  elapsed_ms: 2000,
  defaults: { workers: 2 },
  counts: {
    prepared: 1,
    failed: 0,
    total: 1,
    before_bytes: 30,
    after_bytes: 20,
  },
  history_limit: 256,
};

test("incremental metadata and completion use optimization controls, not saved baseline sizes", () => {
  const events = [
    {
      event_type: "job.started",
      workbook_id: "book",
      worker_index: 1,
      elapsed_millis: 10,
      payload: { name: "book.xlsx" },
    },
    {
      event_type: "workbook.metadata",
      workbook_id: "book",
      elapsed_millis: 20,
      payload: { package: packageSize(100, 1000) },
    },
    {
      event_type: "job.completed",
      workbook_id: "book",
      elapsed_millis: 200,
      payload: {
        archive_sizes: {
          before_optimization: packageSize(30, 300),
          after_optimization: packageSize(20, 100),
        },
      },
    },
  ].map((event) => ({ event }));
  const jobs = applyUpdate([], { ...base, reset: false, events });
  assert.equal(jobs[0].before.file_bytes, 30);
  assert.equal(jobs[0].after.file_bytes, 20);
  const { snapshot } = scenes(base, jobs);
  assert.equal(snapshot.jobs[0].workbookName, "book.xlsx");
  assert.equal(snapshot.jobs[0].workerIndex, 1);
  assert.equal(snapshot.sheets[0].originalBytes, 300);
  assert.equal(snapshot.sheets[0].cleanedBytes, 100);
});

test("conversion has workbook jobs without invented worksheet work", () => {
  const jobs = [
    {
      id: "legacy",
      name: "legacy.xls",
      worker: 0,
      started_ms: 0,
      ended_ms: null,
      status: "running",
      before: null,
      after: null,
      source_bytes: 900,
      error: null,
    },
  ];
  const { snapshot } = scenes({ ...base, operation: "xlsx-convert" }, jobs);
  assert.equal(snapshot.jobs.length, 1);
  assert.equal(snapshot.jobs[0].sheet.name, "legacy.xls");
  assert.equal(snapshot.jobs[0].chunkCount, 1);
  assert.equal(snapshot.sheets.length, 0);
});

test("reconnection replaces stale jobs and retains interrupted outcomes", () => {
  const interrupted = {
    id: "interrupted",
    name: "a.xlsx",
    worker: 0,
    started_ms: 0,
    ended_ms: 30,
    status: "interrupted",
    before: null,
    after: null,
    source_bytes: null,
    error: null,
  };
  const jobs = applyUpdate([{ id: "stale" }], {
    ...base,
    reset: true,
    jobs: [interrupted],
  });
  const { snapshot } = scenes(base, jobs);
  assert.equal(jobs.length, 1);
  assert.equal(snapshot.activeJobs.length, 0);
  assert.equal(snapshot.completedBooks, 0);
});

test("SheetShrink projects real chunk workers, submillisecond times and consistent atlas units", () => {
  const update = {
    ...base,
    operation: "sheet-shrink",
    reset: false,
    events: [
      {
        event: {
          event_type: "work.started",
          workbook_id: "book",
          worker_index: 1,
          elapsed_millis: 0,
          elapsed_micros: 125,
          payload: {
            task_id: 3,
            sheet: "xl/worksheets/sheet1.xml",
            phase: "chunk",
            chunk_index: 2,
            chunk_count: 4,
            range: "cells_range",
          },
        },
      },
      {
        event: {
          event_type: "work.finished",
          workbook_id: "book",
          worker_index: 1,
          elapsed_millis: 0,
          elapsed_micros: 875,
          payload: { task_id: 3 },
        },
      },
    ],
  };
  const tasks = applyTasks([], update);
  const before = { ...packageSize(20, 1000), uncompressed_payload_bytes: 2000 };
  const after = { ...packageSize(10, 500), uncompressed_payload_bytes: 1500 };
  const jobs = [
    {
      id: "book",
      name: "book.xlsx",
      worker: null,
      started_ms: 0,
      ended_ms: 2,
      status: "prepared",
      before,
      after,
      sheet_names: { "xl/worksheets/sheet1.xml": "Measurements" },
    },
  ];
  const { snapshot } = scenes(update, jobs, tasks);
  assert.equal(snapshot.jobs[0].workerIndex, 1);
  assert.equal(snapshot.jobs[0].chunkIndex, 2);
  assert.equal(snapshot.jobs[0].chunkCount, 4);
  assert.equal(snapshot.jobs[0].sheet.name, "Measurements");
  assert.equal(tasks[0].ended_ms - tasks[0].started_ms, 0.75);
  assert.equal(atlasSnapshot(snapshot, jobs).workbooks[0].originalBytes, 2000);
  assert.equal(snapshot.workbooks[0].originalBytes, 20);
});

test("atlas supports seven recent workbooks and descending size order", async () => {
  const bundled = await build({
    entryPoints: ["lib/workbook-scene.ts"],
    bundle: true,
    write: false,
    platform: "node",
    format: "esm",
  });
  const { getAtlasBooks } = await import(
    `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].contents).toString("base64")}`
  );
  const snapshot = {
    time: 100,
    workbooks: Array.from({ length: 10 }, (_, index) => ({
      id: String(index),
      index,
      startAt: index,
      originalBytes: 10 - index,
    })),
  };
  assert.deepEqual(
    getAtlasBooks(snapshot, {
      view: "recent",
      sort: "time",
      recentCount: 7,
    }).map((book) => book.id),
    ["3", "4", "5", "6", "7", "8", "9"],
  );
  assert.deepEqual(
    getAtlasBooks(snapshot, { view: "all", sort: "size" }).map(
      (book) => book.id,
    ),
    ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"],
  );
});
