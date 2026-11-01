  async function probeMillionRowHeights(command, office) {
    const started = performance.now();
    const stats = {batches: 0, requested_ranges: 0};
    const measuredOffice = async action => {
      if (performance.now() - started > command.budget_ms) {
        throw new Error('Experiment measurement deadline exceeded');
      }
      const values = await office(action);
      stats.batches++;
      stats.requested_ranges += values.length;
      return values;
    };
    try {
      const runs = await CaptureGeometry.readHeightRuns(measuredOffice, command.sheet_name, 1048575);
      let next = 1;
      for (const run of runs) {
        if (run.start_row !== next || run.end_row < run.start_row) {
          throw new Error('Incomplete or overlapping height coverage');
        }
        next = run.end_row + 1;
      }
      if (next !== 1048577) throw new Error('Missing final rows');
      return {
        operation: command.operation, complete: true, ...stats,
        elapsed_ms: performance.now() - started, run_count: runs.length,
        total_height: runs.reduce((sum, run) => sum + (run.end_row - run.start_row + 1) * run.height, 0),
        sample_runs: runs.length <= 16 ? runs : [...runs.slice(0, 4), ...runs.slice(-4)],
      };
    } catch (error) {
      return {operation: command.operation, complete: false, ...stats,
        elapsed_ms: performance.now() - started, error: error.message || String(error)};
    }
  }
