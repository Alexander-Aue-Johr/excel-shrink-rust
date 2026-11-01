"""Summarize startup trials and draw their measured loading intervals."""
from datetime import datetime
from html import escape
import json
from pathlib import Path
from statistics import median
import sys


def timestamp(value):
    return datetime.fromisoformat(value.replace('Z', '+00:00')).timestamp()


def concurrent_cpu_seconds(samples):
    """Count intervals with CPU progress in all still-pending Excel processes."""
    overlap = 0.0
    for previous, current in zip(samples, samples[1:]):
        if len(current['clients']) < 2 or len(previous['clients']) != len(current['clients']):
            continue
        pairs = list(zip(previous['clients'], current['clients']))
        if all(not first['ready'] and not second['ready']
               and first['cpu_seconds'] is not None and second['cpu_seconds'] is not None
               and first['pid'] == second['pid']
               and second['cpu_seconds'] - first['cpu_seconds'] >= 0.02
               for first, second in pairs):
            overlap += timestamp(current['at']) - timestamp(previous['at'])
    return overlap


def loading_bars(measurement, maximum):
    clients = measurement['clients']
    origin = min(timestamp(client['requested_at']) for client in clients)
    bars = []
    sequential_offset = 0
    for index, client in enumerate(clients):
        duration = timestamp(client['ready_at']) - timestamp(client['requested_at'])
        start = timestamp(client['requested_at']) - origin
        if measurement['arrangement'] == 'sequential':
            start = sequential_offset
            sequential_offset += duration
        x = 100 + 700 * start / maximum
        width = 700 * duration / maximum
        y = 15 + 34 * index
        color = ['#46c9ad', '#67a9ff', '#efbc67', '#c69cff'][index % 4]
        bars.append(f'<text x="5" y="{y + 17}">Excel {index + 1}</text>'
                    f'<rect x="{x:.2f}" y="{y}" width="{width:.2f}" height="23" fill="{color}">'
                    f'<title>PID {client["pid"]}: {duration:.3f} s</title></rect>')
    height = 27 + 34 * len(clients)
    axis = f'<text x="100" y="{height - 4}">0 s</text><text x="735" y="{height - 4}">{maximum:.1f} s</text>'
    return f'<svg viewBox="0 0 820 {height}" role="img" aria-label="Startup intervals">' + ''.join(bars) + axis + '</svg>'


def batch_seconds(measurement):
    """Read current batch durations and retain support for saved two-workbook results."""
    return measurement['batch_seconds'] if 'batch_seconds' in measurement else measurement['pair_seconds']


def write_report(directory: Path):
    results = json.loads((directory / 'results.json').read_text())
    measurements = results['measurements']
    arrangements = list(dict.fromkeys(item['arrangement'] for item in measurements))
    medians = {name: median(batch_seconds(item) for item in measurements
                           if item['arrangement'] == name) for name in arrangements}
    summary = []
    sections = []
    maximum = max(batch_seconds(item) for item in measurements)
    for measurement in measurements:
        arrangement = measurement['arrangement']
        if arrangement != 'sequential':
            samples_path = directory / f'round-{measurement["repetition"]}-{arrangement}/samples.json'
            samples = json.loads(samples_path.read_text())
            measurement['concurrent_cpu_active_seconds'] = concurrent_cpu_seconds(samples)
            creations = [client['process_started_unix_ms'] for client in measurement['clients']]
            measurement['process_creation_skew_ms'] = max(creations) - min(creations)
        skew = measurement['launch_skew_seconds']
        details = '' if skew is None else f' · launch skew {skew * 1000:.1f} ms'
        if arrangement != 'sequential':
            details += f' · process creation skew {measurement["process_creation_skew_ms"]} ms'
            details += f' · all pending and CPU-active {measurement["concurrent_cpu_active_seconds"]:.1f} s'
        sections.append(
            f'<section><h2>Round {measurement["repetition"]}: {escape(arrangement)}</h2>'
            f'<p>Batch: {batch_seconds(measurement):.3f} s{details}</p>'
            + loading_bars(measurement, maximum) + '</section>')
    for name in arrangements:
        summary.append({'arrangement': name, 'median_batch_seconds': medians[name],
                        'speedup_against_sequential': medians['sequential'] / medians[name]})
    (directory / 'summary.json').write_text(json.dumps({
        'summary': summary, 'measurements': measurements}, indent=2) + '\n')
    rows = ''.join(f'<tr><td>{item["arrangement"]}</td><td>{item["median_batch_seconds"]:.3f} s</td>'
                   f'<td>{item["speedup_against_sequential"]:.3f}×</td></tr>' for item in summary)
    html = '''<!doctype html>
<html lang="en"><meta charset="utf-8"><title>Excel parallel startup experiment</title>
<style>
body { max-width: 1000px; margin: 32px auto; padding: 0 20px; background: #101c28; color: #dae8ef;
       font: 16px/1.5 system-ui, sans-serif; }
section { background: #192c3b; padding: 16px 24px; margin: 18px 0; border-radius: 12px; }
h2 { font-size: 18px; margin: 0; } svg { width: 100%; } svg text { fill: #dae8ef; font-size: 15px; }
td, th { text-align: left; padding: 8px 24px 8px 0; } a { color: #67a9ff; }
</style><h1>Excel parallel startup experiment</h1>'''
    html += (f'<p>{results["sheets"]} sheets × {results["rows_per_sheet"]:,} rows × '
             f'{results["columns"]} numeric columns per workbook.</p>'
             f'<p>{results.get("workbooks_per_trial", 2)} workbooks per trial.</p>'
             '<p>Native process start → verified Office.js add-in readiness. Same machine, '
             'warm filesystem cache, reversed trial order in the second round. '
             'The production corpus was left running.</p>'
             '<table><tr><th>Arrangement</th><th>Median batch time</th><th>Speedup</th></tr>'
             + rows + '</table>' + ''.join(sections)
             + '<p>Bars use a shared scale. Sequential staging/teardown gaps are omitted; '
             'parallel bars retain actual launch offsets. CPU overlap is supporting evidence, '
             'not instrumentation of Excel’s internal parser. These trials do not isolate '
             'taskpane initialization from workbook loading.</p>'
             '<p><a href="results.json">Raw results</a> · <a href="summary.json">Summary and CPU evidence</a></p>'
             '</html>')
    (directory / 'report.html').write_text(html, encoding='utf-8')
    return summary


if __name__ == '__main__':
    print(json.dumps(write_report(Path(sys.argv[1])), indent=2))
