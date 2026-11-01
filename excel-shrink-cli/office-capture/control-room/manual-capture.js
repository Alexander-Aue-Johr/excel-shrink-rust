// Explicit inspections have their own snapshots and never replace corpus results.
export function createManualCaptureControls(container, { current, selectGroup, notice }) {
  const panel = document.createElement('span');
  const index = document.createElement('input');
  index.type = 'number';
  index.min = '1';
  index.value = '1';
  index.style.width = '4rem';
  index.title = 'One-based workbook sheet index';
  index.setAttribute('aria-label', 'Sheet index for manual capture');
  const button = document.createElement('button');
  button.textContent = 'Capture full sheet';
  button.title = 'Capture Original and Default for inspection. This does not change the corpus result.';
  const history = document.createElement('span');
  const back = document.createElement('button');
  back.textContent = 'Back to analysis';
  back.hidden = true;
  let parent = null;
  let busy = false;
  let historyKey = '';
  panel.append(index, button, back, history);
  container.append(panel);
  button.addEventListener('click', async () => {
    const group = current();
    const sheet = Number(index.value);
    if (!group || busy || !Number.isInteger(sheet) || sheet < 1) return;
    busy = true;
    button.disabled = true;
    try {
      const response = await fetch(`/api/groups/${encodeURIComponent(group)}/capture-sheet`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ sheet }),
      });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || `HTTP ${response.status}`);
      parent = group;
      selectGroup(result.id);
      notice('Manual sheet capture started. The corpus analysis result stays unchanged.');
    } catch (error) {
      notice(`Manual capture could not start: ${error.message}`, true);
    } finally {
      busy = false;
    }
  });
  back.addEventListener('click', () => { if (parent) selectGroup(parent); });
  return {
    update(raw) {
      const manual = raw.manual_inspection === true;
      button.disabled = busy || manual;
      index.disabled = manual;
      back.hidden = !manual;
      if (manual) parent = raw.parent_group;
      const captures = raw.manual_captures || [];
      const key = JSON.stringify(captures);
      if (key === historyKey) return;
      historyKey = key;
      history.replaceChildren(...captures.map(capture => {
        const link = document.createElement('button');
        link.textContent = `Inspection · sheet ${capture.sheet}`;
        link.addEventListener('click', () => selectGroup(capture.id));
        return link;
      }));
    },
  };
}
