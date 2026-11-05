/** The host copies an immutable Original before touching the Windows clipboard. */
export function createClipboardButton(groupId, notice) {
  const button = document.createElement('button');
  button.type = 'button';
  button.textContent = 'Copy Original file';
  button.title = 'Copy a separate Original XLSX to the Windows file clipboard. Paste it into your chosen folder with Ctrl+V.';
  button.addEventListener('click', async () => {
    const id = typeof groupId === 'function' ? groupId() : groupId;
    if (!id) return;
    button.disabled = true;
    notice.textContent = 'Preparing an Original inspection copy…';
    try {
      const response = await fetch(`/api/groups/${encodeURIComponent(id)}/copy-original`, { method: 'POST' });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || `HTTP ${response.status}`);
      notice.textContent = 'Original file copied. Paste it into your chosen folder with Ctrl+V. The corpus source is unchanged.';
    } catch (error) { notice.textContent = `Cannot copy Original: ${error.message}`; }
    finally { button.disabled = false; }
  });
  return button;
}
