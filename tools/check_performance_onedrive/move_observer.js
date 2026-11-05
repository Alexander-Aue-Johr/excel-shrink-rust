({batchId, destination, count}) => {
  window.__excelShrinkMoveNotice?.observer.disconnect();
  const readNotices = () => new Map(
    [...document.querySelectorAll('[role="alert"], [role="status"]')].map(element => {
      const text = element.textContent.toLowerCase();
      const visible = element.getClientRects().length > 0 &&
        getComputedStyle(element).visibility !== 'hidden' &&
        element.getAttribute('aria-hidden') !== 'true';
      return [element, {text, visible}];
    })
  );
  let previous = readNotices();
  const state = {batchId, confirmed: false, observer: null};
  const update = () => {
    const current = readNotices();
    for (const [element, notice] of current) {
      const prior = previous.get(element);
      const changed = !prior || prior.text !== notice.text || prior.visible !== notice.visible;
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
    attributeFilter: ['class', 'style', 'hidden', 'aria-hidden']
  });
  window.__excelShrinkMoveNotice = state;
}
