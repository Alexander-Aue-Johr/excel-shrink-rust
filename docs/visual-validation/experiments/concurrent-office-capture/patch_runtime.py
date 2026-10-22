"""Small, checked adapters around the pinned capture runtime; never edit that checkout."""
from pathlib import Path


def replace_once(source, old, new):
    if source.count(old) != 1:
        raise ValueError("Pinned runtime changed at adapter anchor: " + old[:70])
    return source.replace(old, new, 1)


def prepare_web(runtime: Path, destination: Path, experiment: Path):
    import shutil
    if (runtime / "web").resolve() != destination.resolve():
        shutil.copytree(runtime / "web", destination, dirs_exist_ok=True)
    shutil.copyfile(experiment / "web/transport.js", destination / "transport.js")
    path = destination / "taskpane.html"
    html = path.read_text(encoding="utf-8")
    html = replace_once(html, '  <script defer src="/app.js',
                        '  <script defer src="/transport.js"></script>\n  <script defer src="/app.js')
    path.write_text(html, encoding="utf-8")
    path = destination / "app.js"
    source = path.read_text(encoding="utf-8")
    source = replace_once(source, "  async function request(path, body) {",
        """  async function request(path, body) {
    if (path.startsWith('/config')) path += (path.includes('?') ? '&' : '?') +
      'client=' + encodeURIComponent(new URLSearchParams(location.search).get('run'));
    if (path === '/result') body.experiment_metrics = Experiment.metrics;""")
    source = replace_once(source, "    let timer;\n    try {",
                          "    let timer;\n    const began = performance.now();\n    try {")
    source = replace_once(source, "    } finally { clearTimeout(timer); }\n  }\n\n  function bounds",
        "    } finally { clearTimeout(timer); Experiment.record('office_ms', performance.now() - began); }\n  }\n\n  function bounds")
    source = replace_once(source, "    show('workbook', config.workbook);",
        """    Experiment.configure(config, request);
    show('workbook', config.workbook);""")
    source = replace_once(source, "    try {\n      for (let sheet of config.sheets) {",
        """    try {
      await office(async context => {
        context.workbook.load('name');
        await context.sync();
        if (context.workbook.name !== config.workbook) throw new Error('Workbook identity differs');
      });
      await request('/experiment/start', {run_id: config.run_id});
      Experiment.begin();
      for (let sheet of config.sheets) {""")
    first = source.index("    for (let tileIndex = 0; tileIndex < tiles.length; tileIndex++) {")
    last = source.index("      const png = CaptureGeometry.pngSize(base64);", first)
    source = source[:first] + """    for (let offset = 0; offset < tiles.length;) {
      const endBatch = Experiment.batchEnd(tiles, offset, config.batch_size, config.max_batch_pixels);
      const began = performance.now();
      const captured = await office(async context => {
        const worksheet = context.workbook.worksheets.getItem(sheet.name);
        const images = tiles.slice(offset, endBatch).map(tile =>
          worksheet.getRange(CaptureGeometry.address(tile.raw)).getImage());
        await context.sync();
        return images.map(image => image.value);
      });
      Experiment.record('capture_ms', performance.now() - began);
      Experiment.batch(captured.length);
      for (let tileIndex = offset; tileIndex < endBatch; tileIndex++) {
      const tile = tiles[tileIndex];
      const base64 = captured[tileIndex - offset];
""" + source[last:]
    source = replace_once(source, """        await request('/pair/image', {run_id: config.run_id, side: config.side,
          sheet_index: sheet.index, sheet_name: sheet.name, range,
          tile_index: tileIndex, base64: finalPng});""",
        """        await Experiment.upload({run_id: config.run_id, side: config.side,
          sheet_index: sheet.index, sheet_name: sheet.name, range, tile_index: tileIndex}, finalPng);""")
    source = replace_once(source, """    }
    return captures;
""", """      }
      offset = endBatch;
    }
    return captures;
""")
    path.write_text(source, encoding="utf-8")


def process_metrics(source: str):
    return replace_once(source, "    public void Dispose() { job.Dispose(); }", """
    public ulong PeakBytes
    {
        get
        {
            var info = new ExtendedLimits();
            if (!QueryInformationJobObject(job, 9, ref info,
                (uint)Marshal.SizeOf(typeof(ExtendedLimits)), IntPtr.Zero)) throw new Win32Exception();
            return info.PeakJobMemory.ToUInt64();
        }
    }
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool QueryInformationJobObject(SafeFileHandle job, int kind,
        ref ExtendedLimits info, uint size, IntPtr returned);
    [StructLayout(LayoutKind.Sequential)]
    struct Accounting
    {
        public long User, Kernel, PeriodUser, PeriodKernel;
        public uint Faults, Processes, Active, Terminated;
    }
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool QueryInformationJobObject(SafeFileHandle job, int kind,
        ref Accounting info, uint size, IntPtr returned);
    public double CpuSeconds
    {
        get
        {
            var info = new Accounting();
            if (!QueryInformationJobObject(job, 1, ref info,
                (uint)Marshal.SizeOf(typeof(Accounting)), IntPtr.Zero)) throw new Win32Exception();
            return (info.User + info.Kernel) / 10000000.0;
        }
    }
    public void Dispose() { job.Dispose(); }
""")
