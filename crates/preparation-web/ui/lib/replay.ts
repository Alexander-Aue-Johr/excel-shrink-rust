import {
  applyTasks,
  applyUpdate,
  type NativeEvent,
  type Update,
} from "../live";

/** Rebuild only observed state. Seeking never borrows a later workbook report. */
export function replayAt(source: Update, events: NativeEvent[], time: number) {
  const elapsed = Math.max(0, Math.min(source.elapsed_ms, time));
  const update: Update = {
    ...source,
    elapsed_ms: elapsed,
    reset: false,
    history_limit: Number.MAX_SAFE_INTEGER,
    events: events
      .filter(
        (event) =>
          (event.elapsed_micros ?? event.elapsed_millis * 1000) <=
          elapsed * 1000,
      )
      .map((event, cursor) => ({ event, cursor })),
  };
  const jobs = applyUpdate([], update);
  const tasks = applyTasks([], update, Number.MAX_SAFE_INTEGER);
  update.counts = {
    ...source.counts,
    prepared: jobs.filter((job) => job.status === "prepared").length,
    failed: jobs.filter((job) => job.status === "failed").length,
    before_bytes: jobs.reduce(
      (sum, job) => sum + (job.before?.file_bytes ?? 0),
      0,
    ),
    after_bytes: jobs.reduce(
      (sum, job) => sum + (job.after?.file_bytes ?? 0),
      0,
    ),
  };
  return { update, jobs, tasks };
}
