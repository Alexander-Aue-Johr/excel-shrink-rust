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
const { applyUpdate, scenes } = await import(
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
