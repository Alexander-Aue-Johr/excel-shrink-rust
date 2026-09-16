#!/usr/bin/env python3
"""Summarize an excel-shrink Perfetto trace; all times are inclusive wall times."""
import argparse
import collections
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('trace', type=Path)
    args = parser.parse_args()
    trace = json.loads(args.trace.read_text(encoding='utf-8'))
    events = trace['traceEvents']
    for event in events:
        event['name'] = event.get('args', {}).get('phase_name', event.get('name', ''))
    dropped = trace.get('otherData', {}).get('dropped_events', 0)
    print(f'Trace: {args.trace}\nDropped events: {dropped}')
    print('Durations include nested work and scheduling. Do not add overlapping phases.\n')
    durations = collections.defaultdict(list)
    for event in events:
        if event.get('ph') == 'X' and event.get('cat') == 'pipeline':
            durations[event['name']].append(event['dur'] / 1000)
    print(f'{"Phase":45} {"Count":>6} {"Longest ms":>12} {"Sum ms*":>12}')
    for name, values in sorted(durations.items(), key=lambda item: max(item[1]), reverse=True):
        print(f'{name:45} {len(values):6} {max(values):12.2f} {sum(values):12.2f}')
    print('\n* Sum includes concurrent/nested work; it is neither CPU time nor total runtime.')
    drop_times = collections.defaultdict(list)
    drop_before = {e['args']['drop_id'] for e in events if e.get('name') == 'before_drop'}
    drop_after = set()
    for event in events:
        if event.get('name') == 'after_drop':
            drop_after.add(event['args']['drop_id'])
            drop_times[event['args']['resource']].append(event['args']['elapsed_ns'] / 1e6)
    if drop_times:
        print('\nExplicit drops (wall time excluding endpoint emission; aggregate includes children):')
        print(f'{"Resource":50} {"Count":>6} {"Longest ms":>12} {"Sum ms*":>12}')
        for name, values in sorted(drop_times.items(), key=lambda item: max(item[1]), reverse=True):
            print(f'{name:50} {len(values):6} {max(values):12.3f} {sum(values):12.3f}')
    if drop_before != drop_after:
        print(f'\nWARNING: {len(drop_before - drop_after)} drops without after marker; '
              f'{len(drop_after - drop_before)} after markers without before marker.')
    chunks = [e for e in events if e.get('ph') == 'X' and e.get('name') == 'clean_range_chunk']
    chunked_sheets = {(e['args'].get('xlsx'), e['args'].get('sheet')) for e in chunks}
    print(f'\nRange chunking: {len(chunks)} chunks across {len(chunked_sheets)} sheets')
    row_samples = [e['args'] for e in events if e.get('name') == 'range_row_samples']
    if row_samples:
        total_rows = sum(e['total_rows'] for e in row_samples)
        sampled_rows = sum(e['sampled_rows'] for e in row_samples)
        print(f'\nDetailed row samples: {sampled_rows} of {total_rows} rows')
        print('Samples per range/chunk; not an estimate of total CPU or stage time.')
        cell_samples = [e['args'] for e in events if e.get('name') == 'sampled_cell_work']
        stages = collections.Counter()
        for sample in cell_samples:
            for key, value in sample.items():
                if key.endswith('_ns'):
                    stages[key.removesuffix('_ns')] += value
        print(f"Sampled cells: {sum(e['cells'] for e in cell_samples)}; "
              f"changed: {sum(e['changed_cells'] for e in cell_samples)}")
        measured_ns = sum(stages.values())
        for stage, value in stages.most_common():
            share = 100 * value / measured_ns if measured_ns else 0
            print(f'  {stage:38} {value / 1e6:10.3f} ms ({share:5.1f}% of measured cell work)')
    sheets = [e['args'] for e in events if e.get('name') == 'sheet_timing']
    print('\nSheets with longest initial cleanup (milliseconds):')
    for sheet in sorted(sheets, key=lambda s: s['initial_cleanup_ms'], reverse=True)[:10]:
        print(f"  {sheet['sheet']}: initial={sheet['initial_cleanup_ms']}, "
              f"before_finalize={sheet['wait_before_finalize_ms']}, finalize={sheet['finalize_ms']} "
              f"({sheet['xlsx']})")
    peaks = collections.defaultdict(int)
    for event in events:
        if event.get('ph') == 'C':
            peaks[event['name']] = max(peaks[event['name']], event['args']['value'])
    print('\nResource peaks (buffer byte counters are not total process memory):')
    for name, value in sorted(peaks.items()):
        print(f'  {name}: {value}')
    if dropped:
        print('\nWARNING: incomplete trace; timings, flows and resource peaks may be missing.')


if __name__ == '__main__':
    main()
