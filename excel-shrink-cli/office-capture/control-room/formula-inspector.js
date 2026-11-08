/** Read-only formula presentation shared by the live and portable viewers. */
const formulaElement = (tag, className, text) => {
  const element = document.createElement(tag);
  element.className = className;
  if (text != null) element.textContent = text;
  return element;
};

/** A dependency observation is not an assertion that all differences are volatile. */
export function formulaSummary(diagnosis) {
  const randomPaths = diagnosis?.volatile_paths || 0;
  const environmentPaths = diagnosis?.environment_paths || 0;
  if (!randomPaths && !environmentPaths) return '';
  const kinds = [];
  if (randomPaths) kinds.push(`${randomPaths} random/time dependency paths`);
  if (environmentPaths) kinds.push(`${environmentPaths} filename/environment dependency paths`);
  return diagnosis.evidence === 'prepared_source_cached_values'
    ? `${kinds.join(' · ')} in source · captured values unavailable`
    : `${kinds.join(' · ')} · other differences remain unclassified`;
}

/** Split only the verified call span; all formula text stays text, never HTML. */
export function formulaSegments(formula, span) {
  if (!Array.isArray(span) || span.length !== 2 || !span.every(Number.isInteger)
      || span[0] < 0 || span[1] < span[0] || span[1] > formula.length) return [formula, '', ''];
  return [formula.slice(0, span[0]), formula.slice(span[0], span[1]), formula.slice(span[1])];
}

function formulaCode(node, pulsing) {
  const code = formulaElement('code', 'formula-expression');
  const [before, call, after] = formulaSegments(node.formula || '', node.volatile_span ?? node.environment_span);
  code.append(document.createTextNode(before));
  if (call) code.append(formulaElement('span', pulsing ? 'formula-volatile-call' : 'formula-volatile-call pulse-paused', call));
  code.append(document.createTextNode(after));
  return code;
}

/** Use only the retained diagnosis for this exact state pair and region. */
export function retainedFormulaDiagnosis(report, regionKey) {
  const entry = (report.formula_diagnostics?.regions || []).find(entry =>
    `${entry.pair_index}:${entry.kind}:${entry.finding_index}` === regionKey);
  return entry?.diagnosis || {
    evidence: 'unavailable',
    reason: report.formula_diagnostics?.omitted_regions
      ? 'No explanation retained for this region; the recorded diagnostic budget omitted some regions.'
      : 'No formula explanation was retained for this region.',
  };
}

/** Render retained paths without API requests or changes to workbook cells. */
export function renderFormulaDiagnosis(root, region, diagnosis) {
  root.replaceChildren();
  const source = diagnosis.evidence === 'prepared_source_cached_values';
  const note = formulaElement('p', 'formula-evidence-note', diagnosis.reason || diagnosis.note || 'No formula evidence retained.');
  root.append(formulaElement('h3', '', 'Formula dependencies'), note);
  root.classList.toggle('formula-source-only', source);
  if (diagnosis.changed_value_cells != null) {
    root.append(formulaElement('p', '', `${diagnosis.changed_value_cells} changed saved cell values · `
      + `${diagnosis.examined_cells} examined · ${diagnosis.volatile_paths || 0} random/time dependency paths`
      + ` · ${diagnosis.environment_paths || 0} filename/environment dependency paths`
      + (diagnosis.coverage_complete ? '' : ' · partial diagnostic coverage')));
  }
  const examples = (diagnosis.examples || []).filter(example => example.status === 'path_found');
  if (!examples.length) {
    root.append(formulaElement('p', '', 'No supported random/time or filename dependency path is available. This does not prove absence of other dependencies.'));
    return;
  }
  const badge = formulaElement('p', 'formula-dependency-badge', formulaSummary(diagnosis));
  const select = formulaElement('select', '');
  select.setAttribute('aria-label', 'Cell dependency path');
  for (const example of examples) {
    const start = example.path[0];
    const end = example.path.at(-1);
    select.append(formulaElement('option', '', `${start.sheet}!${start.cell} → ${end.cell} · ${example.path.length} cells`));
  }
  const toggle = formulaElement('button', '', 'Pause formula pulse');
  toggle.type = 'button';
  const graph = formulaElement('div', 'formula-path');
  let pulsing = true;
  const render = () => {
    graph.replaceChildren();
    for (const [index, node] of examples[select.selectedIndex].path.entries()) {
      if (index) graph.append(formulaElement('span', 'formula-path-arrow', '→'));
      const card = formulaElement('article', 'formula-cell');
      card.append(formulaElement('strong', '', `${node.sheet}!${node.cell}`));
      card.append(formulaElement('p', 'formula-values', source
        ? `Source cached value: ${node.right_value ?? '(no cached value)'}`
        : `${region.left}: ${node.left_value ?? '(blank)'} → ${region.right}: ${node.right_value ?? '(blank)'}`));
      card.append(formulaCode(node, pulsing));
      if (node.shared_anchor) card.append(formulaElement('small', '', `Shared formula anchor: ${node.shared_anchor}`));
      if (!source && node.left_formula != null && node.left_formula !== node.formula) {
        card.append(formulaElement('p', '', `Left-state formula differs: ${node.left_formula}`));
      }
      graph.append(card);
    }
  };
  select.addEventListener('change', render);
  toggle.addEventListener('click', () => {
    pulsing = !pulsing;
    toggle.textContent = pulsing ? 'Pause formula pulse' : 'Resume formula pulse';
    render();
  });
  root.append(badge, select, toggle, graph);
  render();
}

export function createRetainedFormulaInspector(region, diagnosis) {
  const root = formulaElement('section', 'formula-inspector');
  renderFormulaDiagnosis(root, region, diagnosis);
  return root;
}
