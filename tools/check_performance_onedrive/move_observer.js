({batchId, destination, count}) => {
  window.__excelShrinkMoveNotice?.observer.disconnect();
  const readNotices = () => new Map(
    [...document.querySelectorAll('[role="alert"], [role="status"]')].map(element => {
      const text = element.textContent.toLowerCase();
      const visible = element.getClientRects().length > 0 &&
        getComputedStyle(element).visibility !== 'hidden' &&
        element.getAttribute('aria-hidden') !== 'true';
      const icon = element.matches('[data-automationid="msFluentToast"]')
        ? element.querySelector('[data-icon-name]')?.getAttribute('data-icon-name') : null;
      return [element, {text, visible, icon}];
    })
  );
  let previous = readNotices();
  const state = {batchId, confirmed: false, progressSeen: false, observer: null};
  const update = () => {
    const current = readNotices();
    for (const [element, notice] of current) {
      const prior = previous.get(element);
      const changed = !prior || prior.text !== notice.text || prior.visible !== notice.visible ||
        prior.icon !== notice.icon;
      // Some OneDrive locales render empty toast messages. Require the fresh
      // progress -> completion sequence, never an existing success icon alone.
      if (changed && notice.visible && notice.icon === 'SyncStatusSolid') {
        state.progressSeen = true;
      }
      if (changed && notice.visible && notice.icon === 'CompletedSolid' && state.progressSeen) {
        state.confirmed = true;
      }
      if (changed && notice.visible && notice.text.includes(destination.toLowerCase()) &&
          /\bmoved\b|\bverschoben\b/.test(notice.text) &&
          new RegExp('\\b' + count + '\\b').test(notice.text)) {
        state.confirmed = true;
      }
    }
    previous = current;
  };
  state.observer = new MutationObserver(update);
  state.observer.observe(document.body, {
    subtree: true, childList: true, characterData: true, attributes: true,
    attributeFilter: ['class', 'style', 'hidden', 'aria-hidden', 'data-icon-name']
  });
  window.__excelShrinkMoveNotice = state;
}
