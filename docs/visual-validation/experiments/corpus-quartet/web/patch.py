"""Attach the quartet benchmark to an exported, pinned Office capture runtime."""

from pathlib import Path
import shutil


def replace_once(source: str, old: str, new: str) -> str:
    if source.count(old) != 1:
        raise ValueError(f"Capture runtime adapter anchor changed: {old[:70]!r}")
    return source.replace(old, new, 1)


def patch_runtime(runtime: Path) -> None:
    runtime = Path(runtime)
    web = next((candidate for candidate in (
        runtime, runtime / "web", runtime / "excel-shrink-cli/office-capture/web")
        if (candidate / "app.js").exists()), None)
    if web is None:
        raise FileNotFoundError(f"Exported Office capture runtime missing: {web}")
    shutil.copyfile(Path(__file__).with_name("transport.js"), web / "transport.js")

    taskpane = web / "taskpane.html"
    html = taskpane.read_text(encoding="utf-8")
    if 'src="/transport.js"' not in html:
        html = replace_once(html, '  <script defer src="/app.js',
            '  <script defer src="/transport.js"></script>\n  <script defer src="/app.js')
        taskpane.write_text(html, encoding="utf-8")

    app = web / "app.js"
    source = app.read_text(encoding="utf-8")
    already_adapted = "Experiment.configure(config, request);" in source
    if not already_adapted:
        source = replace_once(source, "  async function request(path, body) {", """  async function request(path, body) {
    if (path.startsWith('/config')) path += (path.includes('?') ? '&' : '?') +
      'client=' + encodeURIComponent(new URLSearchParams(location.search).get('run'));
    if (path === '/result') body.experiment_metrics = Experiment.metrics;""")
    source = replace_once(source,
        "    const timer = setTimeout(() => abort.abort(), requestTimeoutMs);",
        """    const barrier = ['/experiment/preflight', '/experiment/start', '/pair/geometry', '/round',
      '/refine', '/pair/sheet'].includes(path);
    const timer = barrier ? null : setTimeout(() => abort.abort(), requestTimeoutMs);""")
    if not already_adapted:
        source = replace_once(source, "    let timer;\n    try {",
            "    let timer;\n    const began = performance.now();\n    try {")
        source = replace_once(source, "    } finally { clearTimeout(timer); }\n  }\n\n  function bounds",
            "    } finally { clearTimeout(timer); Experiment.record('office_ms', performance.now() - began); }\n  }\n\n  function bounds")

    start = source.index("  async function captureSheet(config, sheet) {")
    end = source.index("\n  async function run() {", start)
    source = source[:start] + """  async function captureSheet(config, sheet) {
    return Experiment.captureSheet(sheet);
  }
""" + source[end:]
    configure = """    Experiment.configure(config, request, {
      office, bounds, targetRows, checkedWorksheet, hideGridlines, verifyDrawingBounds
    });"""
    if already_adapted:
        source = replace_once(source, "    Experiment.configure(config, request);", configure)
        source = replace_once(source,
            "      await request('/experiment/start', {run_id: config.run_id});\n      Experiment.begin();",
            """      phase('Waiting for stable task pane');
      await request('/experiment/preflight', {run_id: config.run_id});
      phase('Preparing capture geometry');
      await Experiment.prepareAll(config.sheets);
      await request('/ready', {run_id: config.run_id});
      phase('Waiting at common start line');
      await request('/experiment/start', {run_id: config.run_id});
      Experiment.begin();""")
    else:
        source = replace_once(source, "    show('workbook', config.workbook);",
            configure + "\n    show('workbook', config.workbook);")
        source = replace_once(source, "    try {\n      for (let sheet of config.sheets) {", """    try {
      await office(async context => {
        context.workbook.load('name');
        await context.sync();
        if (context.workbook.name !== config.workbook) throw new Error('Workbook identity differs');
      });
      phase('Waiting for stable task pane');
      await request('/experiment/preflight', {run_id: config.run_id});
      phase('Preparing capture geometry');
      await Experiment.prepareAll(config.sheets);
      await request('/ready', {run_id: config.run_id});
      phase('Waiting at common start line');
      await request('/experiment/start', {run_id: config.run_id});
      Experiment.begin();
      for (let sheet of config.sheets) {""")
    source = replace_once(source,
        "        await checkedWorksheet(sheet);\n        if (config.mode === 'paired' && config.check_heights) {",
        "        if (config.mode === 'paired' && config.check_heights) {")
    app.write_text(source, encoding="utf-8")
