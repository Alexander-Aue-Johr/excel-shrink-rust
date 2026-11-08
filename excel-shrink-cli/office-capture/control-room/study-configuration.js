/* Archive-study configuration and a verified handoff to its own Observatory. */

export function configurationFromForm(form, defaults) {
  return Object.fromEntries(Object.entries(defaults).map(([name, initial]) => {
    const field = form.elements.namedItem(name);
    if (!field) throw new Error(`Missing study field ${name}`);
    let value;
    if (typeof initial === 'boolean') value = field.checked;
    else if (typeof initial === 'number') {
      if (!field.value.trim()) throw new Error(`Missing integer for ${name}`);
      value = Number(field.value);
      if (!Number.isSafeInteger(value)) throw new Error(`Invalid integer for ${name}`);
    } else value = field.value.trim();
    return [name, value];
  }));
}

export function partitionChoices(count) {
  if (!Number.isSafeInteger(count) || count < 1 || count > 1000) {
    throw new Error('Choose between 1 and 1000 size partitions');
  }
  return Array.from({ length: count + 1 }, (_, index) => index);
}

export function installStudyConfiguration({ api, message, defaults, logicalProcessors }) {
  const byId = id => document.getElementById(id);
  const form = byId('new-study');
  for (const [name, initial] of Object.entries(defaults)) {
    const field = form.elements.namedItem(name);
    if (!field) throw new Error(`Missing study field ${name}`);
    if (typeof initial === 'boolean') field.checked = initial;
    else field.value = initial;
  }
  for (const name of ['prepare_processes', 'shrink_threads']) {
    form.elements.namedItem(name).max = logicalProcessors;
  }

  function renderPartitions() {
    const selected = form.elements.namedItem('partition');
    const count = Number(form.elements.namedItem('partitions').value);
    if (!Number.isSafeInteger(count) || count < 1 || count > 1000) return;
    if (Number(selected.value) > count) selected.value = 0;
    byId('study-partitions').replaceChildren(...partitionChoices(count).map(index => {
      const button = document.createElement('button');
      button.type = 'button';
      button.textContent = index === 0 ? 'All files' : `${index} / ${count}`;
      button.setAttribute('aria-pressed', String(index === Number(selected.value)));
      button.addEventListener('click', () => {
        selected.value = index;
        renderPartitions();
      });
      return button;
    }));
  }
  form.elements.namedItem('partitions').addEventListener('change', renderPartitions);
  renderPartitions();

  async function handoff(result, linkId) {
    const link = byId(linkId);
    link.href = result.url;
    link.hidden = false;
    message(`Study runner started with PID ${result.pid}. Waiting for its verified Observatory.`);
    for (let attempt = 0; attempt < 30; attempt += 1) {
      const status = await api('/api/study/ready');
      if (status.ready) {
        window.location.assign(status.url);
        return;
      }
      await new Promise(resolve => setTimeout(resolve, 1000));
    }
    message('The study is still starting. Its Observatory link remains available.');
  }

  form.addEventListener('submit', async event => {
    event.preventDefault();
    const button = byId('start-study');
    button.disabled = true;
    try {
      const result = await api('/api/study/start', configurationFromForm(form, defaults));
      await handoff(result, 'new-study-link');
    } catch (error) {
      message(error.message, true);
    } finally {
      button.disabled = false;
    }
  });

  byId('resume-study').addEventListener('click', async () => {
    const button = byId('resume-study');
    button.disabled = true;
    try {
      const result = await api('/api/study/resume', { receipt: byId('study-receipt').value.trim() });
      await handoff(result, 'study-link');
    } catch (error) {
      message(error.message, true);
    } finally {
      button.disabled = false;
    }
  });
}
