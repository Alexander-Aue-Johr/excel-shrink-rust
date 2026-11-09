export function acquisitionRequest(form, operation) {
  if (!['inspect', 'prepare'].includes(operation)) throw new Error('Choose a corpus operation.');
  const value = name => form.elements.namedItem(name).value.trim();
  const request = {
    profile: value('profile'), operation, artifacts: value('artifacts'),
    cache: value('cache'), archive: value('archive') || null,
  };
  if (!request.profile || !request.artifacts || !request.cache) {
    throw new Error('Choose a corpus, originals directory and download cache.');
  }
  return request;
}

export function acquisitionStatus(job) {
  if (!job) return '';
  const progress = job.progress || {};
  const parts = [job.request.profile, job.status, progress.phase?.replaceAll('_', ' ')];
  if (progress.total != null && progress.added != null) {
    parts.push(`${progress.added} restored of ${progress.total} files`);
  }
  if (progress.member) parts.push(progress.member);
  return parts.filter(Boolean).join(' · ');
}

export async function installCorpusAcquisition({ api, message, session }) {
  const byId = id => document.getElementById(id);
  const form = byId('corpus-acquisition');
  const catalog = await api('/api/corpora');
  const select = byId('corpus-profile');
  select.replaceChildren(...catalog.profiles.map(profile => {
    const option = document.createElement('option');
    option.value = profile.id;
    option.textContent = profile.acquisition_ready === false
      ? `${profile.name} (awaiting extraction package)` : profile.name;
    option.disabled = profile.acquisition_ready === false;
    return option;
  }));
  byId('corpus-cache').value = session.corpus_download_cache;
  let suggestedArtifacts = '';
  function selectedProfile() {
    const profile = catalog.profiles.find(item => item.id === select.value);
    if (!profile) return;
    if (profile.acquisition_ready === false) {
      byId('corpus-profile-details').textContent = `${profile.archive_count} source archives · extraction recipe and workbook manifest pending`;
      return;
    }
    const artifacts = byId('corpus-artifacts');
    if (!artifacts.value || artifacts.value === suggestedArtifacts) {
      suggestedArtifacts = `${session.corpus_artifacts_root.replace(/[\\/]+$/, '')}/${profile.id}`;
      artifacts.value = suggestedArtifacts;
    }
    byId('corpus-profile-details').textContent =
      `${profile.workbook_count.toLocaleString()} workbooks · archive ${(profile.archive.bytes / 2 ** 30).toFixed(2)} GiB\n`
      + `${profile.archive.url}\nSHA-256 ${profile.archive.sha256}`;
  }
  select.addEventListener('change', selectedProfile);
  selectedProfile();

  let submitting = false;
  function render(job, active) {
    byId('corpus-fields').disabled = submitting || Boolean(active);
    byId('corpus-observation').hidden = !job;
    byId('corpus-job-status').textContent = acquisitionStatus(job);
    byId('corpus-job-result').textContent = job?.error
      || (job?.result ? JSON.stringify(job.result, null, 2) : '');
  }
  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (submitting) return;
    submitting = true;
    try {
      const request = acquisitionRequest(form, event.submitter?.value || 'inspect');
      byId('corpus-fields').disabled = true;
      const result = await api('/api/corpora/start', request);
      render(result.job, true);
      message('Corpus operation started. Progress remains available after closing this configuration dialog.');
    } catch (error) {
      byId('corpus-fields').disabled = false;
      message(error.message, true);
    } finally {
      submitting = false;
    }
  });
  return { render };
}
