/* Portable package controls do not start native preparation or visual validation. */
export function importRequest(form) {
  const value = name => form.elements.namedItem(name).value.trim();
  const packagePath = value('package');
  const sourceManifest = value('source_manifest');
  const output = value('output');
  if (!packagePath || !sourceManifest || !output) {
    throw new Error('Choose a package, source manifest and new import directory.');
  }
  return {
    package: packagePath,
    source_manifest: sourceManifest,
    output,
    generation_choices: value('generation_choices') || null,
    adopt_packaged_baseline: form.elements.namedItem('adopt_packaged_baseline').checked,
  };
}

export function installPreparationPackages({ api, message, onImported }) {
  const byId = id => document.getElementById(id);
  const form = byId('preparation-import');
  const exportForm = byId('preparation-export');
  let offset = 0;
  let imported = null;
  let busy = false;
  let page = null;

  async function operation(work) {
    if (busy) return;
    busy = true;
    const controls = [...document.querySelectorAll('#preparation-packages button, #preparation-packages input')];
    const disabled = controls.map(control => control.disabled);
    controls.forEach(control => { control.disabled = true; });
    try {
      await work();
    } catch (error) {
      message(error.message, true);
    } finally {
      controls.forEach((control, index) => { control.disabled = disabled[index]; });
      byId('preparation-previous').disabled = !page || offset === 0;
      byId('preparation-next').disabled = !page || offset + 100 >= page.selected;
      busy = false;
    }
  }

  async function preview(nextOffset) {
    const packagePath = form.elements.namedItem('package').value.trim();
    const sourceManifest = form.elements.namedItem('source_manifest').value.trim();
    if (!packagePath || !sourceManifest) throw new Error('Choose a package and source manifest.');
    message('Verifying package and selected source coverage…');
    const result = await api('/api/preparation/preview', {
      package: packagePath, source_manifest: sourceManifest, offset: nextOffset, limit: 100,
    });
    offset = result.offset;
    page = result;
    byId('preparation-summary').textContent = `${result.selected} selected. ${Object.entries(result.counts).map(([status, count]) => `${status.replaceAll('_', ' ')} ${count}`).join(' · ')}`;
    byId('preparation-items').replaceChildren(...result.items.map(item => {
      const details = document.createElement('details');
      const summary = document.createElement('summary');
      summary.textContent = `${item.name} · ${item.status.replaceAll('_', ' ')}`;
      const evidence = document.createElement('pre');
      evidence.textContent = JSON.stringify({ source_id: item.source_id, detail: item.detail, candidates: item.candidates }, null, 2);
      details.append(summary, evidence);
      return details;
    }));
    message('Coverage inspected. Import requires explicit resolution of every selected occurrence.');
  }

  byId('preview-preparation').addEventListener('click', () => operation(() => preview(0)));
  byId('preparation-previous').addEventListener('click', () => operation(() => preview(Math.max(0, offset - 100))));
  byId('preparation-next').addEventListener('click', () => operation(() => preview(offset + 100)));
  form.addEventListener('submit', event => {
    event.preventDefault();
    void operation(async () => {
      imported = null;
      byId('use-preparation').hidden = true;
      message('Verifying and copying selected preparation states…');
      const result = await api('/api/preparation/import', importRequest(form));
      imported = result.configuration;
      byId('use-preparation').hidden = false;
      message(`${result.imported} pairs imported. Review the comparison configuration before starting validation.`);
    });
  });
  byId('use-preparation').addEventListener('click', () => { if (imported) onImported(imported); });
  form.addEventListener('change', () => {
    imported = null;
    page = null;
    byId('use-preparation').hidden = true;
    byId('preparation-previous').disabled = true;
    byId('preparation-next').disabled = true;
  });
  exportForm.addEventListener('submit', event => {
    event.preventDefault();
    void operation(async () => {
      message('Exporting verified preparation package…');
      const result = await api('/api/preparation/export', {
        preparation: exportForm.elements.namedItem('preparation').value.trim(),
        output: exportForm.elements.namedItem('output').value.trim(),
      });
      message(`${result.exported} preparation outcomes exported to ${result.output}.`);
    });
  });
}
