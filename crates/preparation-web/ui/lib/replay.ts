import {
  applyTasks,
  applyUpdate,
  type NativeEvent,
  type Job,
  type WorkTask,
  type Update,
} from "../live";

/** Rebuild only observed state. Seeking never borrows a later workbook report. */
export function replayAt(source: Update, events: NativeEvent[], time: number) {
  return createReplay(source, events)(time);
}

/** Forward playback applies new events once. Backward seeks rebuild observed state. */
export function createReplay(source: Update, events: NativeEvent[]) {
  let cursor = 0;
  let previousTime = -1;
  let jobs: Job[] = [];
  let tasks: WorkTask[] = [];

  return (time: number) => {
    const elapsed = Math.max(0, Math.min(source.elapsed_ms, time));
    if (elapsed < previousTime) {
      cursor = 0;
      jobs = [];
      tasks = [];
    }
    const pending: Update["events"] = [];
    while (cursor < events.length) {
      const event = events[cursor];
      if (
        (event.elapsed_micros ?? event.elapsed_millis * 1000) >
        elapsed * 1000
      )
        break;
      pending.push({ event, cursor });
      cursor += 1;
    }
    const update: Update = {
      ...source,
      elapsed_ms: elapsed,
      reset: false,
      history_limit: Number.MAX_SAFE_INTEGER,
      events: pending,
    };
    if (pending.length > 0) {
      jobs = applyUpdate(jobs, update);
      tasks = applyTasks(tasks, update, Number.MAX_SAFE_INTEGER);
    }
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
    previousTime = elapsed;
    return { update, jobs, tasks };
  };
}
