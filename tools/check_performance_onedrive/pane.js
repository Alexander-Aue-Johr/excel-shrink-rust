async () => {
  const optimizeLabel = /^(Optimize all|Alle optimieren)$/i;
  const checkAgainLabel = /^(Check again|Erneut prüfen)$/i;
  const optimizedMessage = /Workbook optimized|Arbeitsmappe optimiert/i;
  const alreadyCleanMessage = /No optimization (?:required|needed)|Keine Optimierung (?:erforderlich|nötig)/i;
  const optimizingLabel = /Optimizing|Wird optimiert/i;

  const visibleButton = (label) => [...document.querySelectorAll("button")]
    .find((button) => button.getClientRects().length > 0
      && label.test(button.innerText.trim()));
  const optimizeButton = visibleButton(optimizeLabel);

  if (alreadyCleanMessage.test(document.body.innerText)) {
    return "already_clean";
  }
  if (!optimizeButton) {
    return "optimize_button_missing";
  }

  return await new Promise((resolve) => {
    let accepted = false;
    let checkedAgain = false;
    let scheduled = false;
    let finished = false;
    let notifications = Promise.resolve();

    const notify = (phase) => {
      notifications = notifications.then(() => window.__checkPerformanceEvent(phase));
    };
    const finish = (outcome) => {
      if (finished) return;
      finished = true;
      observer.disconnect();
      clearTimeout(acceptanceTimeout);
      clearTimeout(completionTimeout);
      notifications.then(() => resolve(outcome), () => resolve("telemetry_failed"));
    };
    const accept = () => {
      if (accepted) return;
      accepted = true;
      notify("optimization_accepted");
    };
    const inspect = () => {
      scheduled = false;
      if (finished) return;

      const text = document.body.innerText;
      if (optimizedMessage.test(text)) {
        accept();
        finish("optimized");
        return;
      }
      if (alreadyCleanMessage.test(text)) {
        accept();
        finish(checkedAgain ? "already_clean_after_check_again" : "already_clean");
        return;
      }

      const checkAgain = visibleButton(checkAgainLabel);
      if (checkAgain) {
        if (!checkedAgain) {
          checkedAgain = true;
          checkAgain.click();
          notify("check_again_clicked");
        }
        return;
      }

      const currentButton = visibleButton(optimizeLabel);
      if (checkedAgain && currentButton) {
        finish("optimization_not_applied");
        return;
      }
      if (!accepted && (optimizeButton.disabled
          || optimizeButton.getAttribute("aria-disabled") === "true"
          || optimizingLabel.test(optimizeButton.innerText))) {
        accept();
      }
    };
    const scheduleInspect = () => {
      if (scheduled || finished) return;
      scheduled = true;
      queueMicrotask(inspect);
    };
    const observer = new MutationObserver(scheduleInspect);
    observer.observe(document.body, {
      childList: true,
      characterData: true,
      attributes: true,
      attributeFilter: ["disabled", "aria-disabled", "class", "style"],
      subtree: true,
    });
    const acceptanceTimeout = setTimeout(() => {
      if (!accepted) finish("click_not_acknowledged");
    }, 20_000);
    const completionTimeout = setTimeout(() => finish("optimization_timed_out"), 180_000);

    optimizeButton.click();
    notify("optimization_click_dispatched");
    scheduleInspect();
  });
}
