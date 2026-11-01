"""Rank nominal CF fallback regions without claiming actual Excel capture times."""
import json
from pathlib import Path
import sys


def text(value) -> str:
    return str(value).replace('|', '\\|').replace('\n', ' ')


def report(root: Path) -> None:
    summary = json.loads((root / 'summary.json').read_text(encoding='utf-8'))
    largest = []
    with (root / 'sheets.jsonl').open(encoding='utf-8') as records:
        for line in records:
            sheet = json.loads(line)
            for index, block in enumerate(sheet['blocks']):
                if block['fallback_reason'].startswith('eligible-'):
                    continue
                largest.append((block['tile_proxy_upper_bound'] or 0, sheet['source_id'],
                                sheet['workbook'], sheet['part'], index, block))
                largest.sort(key=lambda item: item[0], reverse=True)
                del largest[30:]
    lines = [
        '# Conditional formatting inventory', '',
        f'Status: **{summary["status"]}**. {summary["workbooks"]}/{summary["total_workbooks"]} workbooks; '
        f'{summary["sheets"]} worksheet XML entries; {summary["blocks"]} CF blocks; '
        f'{summary["errors"]} errors; {summary["elapsed_seconds"]:.2f} seconds.', '',
        'Eligibility is not an invariance proof. No Original/Default comparison was performed. '
        'Costs use the fixed row/column grid in settings.json, without clipping to visible content. '
        'Overlapping ranges and blocks are counted repeatedly; these are ranking proxies, not actual capture counts or timings.', '',
        '| Classification | CF blocks | Nominal tile proxy | Unknown ranges |',
        '| --- | ---: | ---: | ---: |',
    ]
    for key, value in sorted(summary['by_fallback'].items(), key=lambda pair: pair[1]['tile_proxy_upper_bound'], reverse=True):
        lines.append(f'| {text(key)} | {value["blocks"]} | {value["tile_proxy_upper_bound"]} | {value["unknown_ranges"]} |')
    lines += ['', '## Largest unclassified or unsupported regions', '']
    for cost, source_id, name, part, index, block in largest:
        lines += [f'### {text(name)}', '',
                  f'- Source ID: `{source_id}`; part: `{part}`; block index: `{index}`.',
                  f'- Range: `{text(block["sqref"])}`; nominal proxy: **{cost}**.',
                  f'- Reason: `{block["fallback_reason"]}`.',
                  f'- Cached workbook: [sources/{source_id:05}.xlsx](sources/{source_id:05}.xlsx).']
        for rule in block['rules']:
            lines.append(f'- Rule: `{text(rule["attributes"])}`; formulas: `{text(rule["formulas"])}`; `{rule["classification"]}`.')
        lines.append('')
    (root / 'report.md').write_text('\n'.join(lines), encoding='utf-8')
    (root / 'largest-unsupported.json').write_text(json.dumps(largest, indent=2, ensure_ascii=False), encoding='utf-8')
    print(root / 'report.md')


if __name__ == '__main__':
    report(Path(sys.argv[1]))
