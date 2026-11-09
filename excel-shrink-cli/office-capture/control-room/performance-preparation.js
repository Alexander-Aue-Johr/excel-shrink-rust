export function preparationJobRequest(form) {
  const value = name => form.elements.namedItem(name).value.trim();
  const workers = Number(value('workers'));
  if (!Number.isSafeInteger(workers) || workers < 1) throw new Error('Choose a positive integer worker count.');
  const output = value('output');
  if (!output) throw new Error('Choose a new preparation directory.');
  const request = { output, workers, reuse_from: value('reuse_from') || null };
  const kind = form.elements.namedItem('source_kind')?.value || 'manifest';
  if (kind === 'directory') {
    const directory = value('source_directory');
    if (!directory) throw new Error('Choose an XLSX originals directory.');
    return { ...request, source_manifest: null, source_directory: directory, corpus_profile: value('corpus_profile') || null };
  }
  if (kind !== 'manifest') throw new Error('Choose a supported input selection.');
  const sourceManifest = value('source_manifest');
  if (!sourceManifest) throw new Error('Choose a source manifest.');
  return { ...request, source_manifest: sourceManifest };
}

export function installPreparationSources({ api, message }) {
  const byId = id => document.getElementById(id);
  const form = byId('performance-job');

  function selectSourceKind() {
    const directory = byId('performance-source-kind').value === 'directory';
    byId('performance-directory-fields').hidden = !directory;
    byId('performance-manifest-field').hidden = directory;
    form.elements.namedItem('source_directory').required = directory;
    form.elements.namedItem('source_manifest').required = !directory;
  }
  byId('performance-source-kind').addEventListener('change', selectSourceKind);
  selectSourceKind();
  api('/api/corpora').then(catalog => {
    for (const profile of catalog.profiles.filter(profile => profile.acquisition_ready !== false)) {
      const option = document.createElement('option');
      option.value = profile.id;
      option.textContent = profile.name;
      byId('performance-corpus-profile').append(option);
    }
  }).catch(error => message(`Preparation corpus profiles unavailable. ${error.message}`, true));
}

export function installPerformancePreparation({ api, message }) {
  const byId = id => document.getElementById(id);
  const form = byId('performance-job');
  let busy = false;
  let currentJob = null;
  let changingWorkers = false;

  function render(job, active) {
    // Restore the selected job after page load without overwriting an in-progress edit on polling.
    if (job && job.id !== currentJob?.id) {
      byId('performance-worker-target').value = String(job.progress?.requested_workers ?? job.workers);
    }
    currentJob = job;
    byId('performance-worker-control').hidden = !job || job.status !== 'running';
    byId('performance-worker-apply').disabled = changingWorkers;
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
      `${counters.requested_workers ?? job.workers} requested workers`,
      `${counters.in_flight || 0} in-flight requests`,
      `${counters.queued ?? total} queued`,
      job.output,
      job.error,
      job.progress_error,
    ].filter(value => value != null).join(' · ');
  }

  byId('performance-worker-control').addEventListener('submit', async event => {
    event.preventDefault();
    if (changingWorkers || currentJob?.status !== 'running') return;
    const workers = Number(byId('performance-worker-target').value);
    if (!Number.isSafeInteger(workers) || workers < 1) {
      message('Choose a positive integer worker count.', true);
      return;
    }
    changingWorkers = true;
    byId('performance-worker-apply').disabled = true;
    try {
      await api('/api/preparation/control', { job_id: currentJob.id, workers });
      message(`Preparation worker target set to ${workers}. Active workbooks finish before retirement.`);
    } catch (error) {
      message(error.message, true);
    } finally {
      changingWorkers = false;
      byId('performance-worker-apply').disabled = false;
    }
  });

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
