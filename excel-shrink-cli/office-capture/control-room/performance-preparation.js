export function preparationJobRequest(form) {
  const value = name => form.elements.namedItem(name).value.trim();
  const workers = Number(value('workers'));
  if (!Number.isSafeInteger(workers) || workers < 1) throw new Error('Choose a positive integer worker count.');
  const sourceManifest = value('source_manifest');
  const output = value('output');
  if (!sourceManifest || !output) throw new Error('Choose a source manifest and a new preparation directory.');
  return { source_manifest: sourceManifest, output, workers, reuse_from: value('reuse_from') || null };
}

export function installPerformancePreparation({ api, message }) {
  const byId = id => document.getElementById(id);
  const form = byId('performance-job');
  let busy = false;

  function render(job, active) {
    byId('performance-job-fields').disabled = busy || Boolean(active);
    byId('performance-job-observation').hidden = !job;
    if (!job) return;
    const counters = job.progress || {};
    const finished = (counters.prepared || 0) + (counters.failed || 0);
    const total = counters.total ?? job.total;
    byId('performance-job-status').textContent = `${job.status} · ${finished} / ${total} finished · ${counters.prepared || 0} prepared (${counters.reused || 0} reused) · ${counters.failed || 0} failed`;
    const progress = byId('performance-job-progress');
    progress.max = Math.max(1, total);
    progress.value = finished;
    byId('performance-job-detail').textContent = [
      `${job.workers} requested workers`,
      `${counters.in_flight || 0} in-flight requests`,
      `${counters.queued ?? total} queued`,
      job.output,
      job.error,
      job.progress_error,
    ].filter(value => value != null).join(' · ');
  }

  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (busy) return;
    busy = true;
    byId('performance-job-fields').disabled = true;
    try {
      const result = await api('/api/preparation/start', preparationJobRequest(form));
      render(result.job, true);
      message('Check Performance preparation started. It continues independently of this browser tab.');
    } catch (error) {
      byId('performance-job-fields').disabled = false;
      message(error.message, true);
    } finally {
      busy = false;
    }
  });
  return { render };
}
