"""Rank nominal CF fallback regions without claiming actual Excel capture times."""
import json
import html
from collections import Counter, defaultdict
from pathlib import Path
import sys


def text(value) -> str:
    return str(value).replace('|', '\\|').replace('\n', ' ')


def storage_kind(block) -> str:
    if block['raw_xml'] is not None:
        return 'inline'
    if block['raw_file'] is not None:
        return 'external'
    return 'source_only'


def collect_evidence(root: Path):
    """Stream worksheet records, retaining aggregate counts and only the largest examples."""
    largest = []
    families = defaultdict(lambda: {'blocks': 0, 'tiles': 0, 'workbooks': set(), 'sources': set()})
    storage = Counter()
    unknown = []
    rule_count = 0
    max_node_bytes = 0
    with (root / 'sheets.jsonl').open(encoding='utf-8') as records:
        for line in records:
            sheet = json.loads(line)
            for index, block in enumerate(sheet['blocks']):
                storage[storage_kind(block)] += 1
                rule_count += len(block['rules'])
                max_node_bytes = max(max_node_bytes, block['end'] - block['start'])
                if block['tile_proxy_upper_bound'] is None:
                    unknown.append({'source': sheet['source_id'], 'workbook': sheet['workbook'],
                                    'part': sheet['part'], 'block': index, 'sqref': block['sqref'],
                                    'raw_xml': block['raw_xml']})
                if block['fallback_reason'].startswith('eligible-'):
                    continue
                # A display grouping, not a parser or proof of formula equivalence.
                formulas = [html.unescape(formula) for rule in block['rules'] for formula in rule['formulas']]
                kinds = sorted({rule['attributes'].get('type', 'missing') for rule in block['rules']})
                family = 'INDIRECT comparisons' if any('INDIRECT(' in formula.upper() for formula in formulas) else '+'.join(kinds)
                group = families[family]
                group['blocks'] += 1
                group['tiles'] += block['tile_proxy_upper_bound'] or 0
                group['workbooks'].add(sheet['workbook'])
                group['sources'].add(sheet['source_id'])
                largest.append((block['tile_proxy_upper_bound'] or 0, sheet['source_id'],
                                sheet['workbook'], sheet['part'], index, block))
                largest.sort(key=lambda item: item[0], reverse=True)
                del largest[30:]
    ranked = []
    for family, group in families.items():
        ranked.append({'family': family, **group,
                       'workbooks': sorted(group['workbooks']), 'sources': sorted(group['sources'])})
    ranked.sort(key=lambda group: group['tiles'], reverse=True)
    analysis = {'rule_count': rule_count, 'storage': dict(storage), 'max_node_bytes': max_node_bytes,
                'source_cache_bytes': sum(path.stat().st_size for path in (root / 'sources').glob('*.xlsx')),
                'unsupported_families': ranked, 'unknown_ranges': unknown}
    return analysis, largest


def render_report(summary, analysis, largest) -> str:
    """Keep estimated costs, unknown ranges and proven observations visibly distinct."""
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
    lines += ['', '## Unsupported rule families', '',
              'Each block is grouped once. Blocks containing INDIRECT are grouped first; otherwise the group is the set of rule types. '
              'This is a display grouping, not a semantic proof.', '',
              '| Family | Blocks | Workbooks | Nominal tile proxy |',
              '| --- | ---: | ---: | ---: |']
    for group in analysis['unsupported_families']:
        lines.append(f'| {text(group["family"])} | {group["blocks"]} | {len(group["workbooks"])} | {group["tiles"]} |')
    lines += ['', '## Unknown application ranges', '',
              'These require separate review; an unknown range is not a zero-cost or safe case.', '']
    for item in analysis['unknown_ranges']:
        lines.append(f'- `{text(item["workbook"])}` · source `{item["source"]}` · `{item["part"]}` · block `{item["block"]}`.')
    lines += ['', '## Evidence', '',
              f'{analysis["rule_count"]} individual rules; storage: `{analysis["storage"]}`. Largest raw node: {analysis["max_node_bytes"]} bytes. '
              f'Retained XLSX bytes: {analysis["source_cache_bytes"]}.', '',
              '## Largest unclassified or unsupported regions', '']
    for cost, source_id, name, part, index, block in largest:
        lines += [f'### {text(name)}', '',
                  f'- Source ID: `{source_id}`; part: `{part}`; block index: `{index}`.',
                  f'- Range: `{text(block["sqref"])}`; nominal proxy: **{cost}**.',
                  f'- Reason: `{block["fallback_reason"]}`.',
                  f'- Cached workbook: [sources/{source_id:05}.xlsx](sources/{source_id:05}.xlsx).']
        for rule in block['rules']:
            lines.append(f'- Rule: `{text(rule["attributes"])}`; formulas: `{text(rule["formulas"])}`; `{rule["classification"]}`.')
        lines.append('')
    return '\n'.join(lines)


def report(root: Path) -> None:
    summary = json.loads((root / 'summary.json').read_text(encoding='utf-8'))
    analysis, largest = collect_evidence(root)
    (root / 'report.md').write_text(render_report(summary, analysis, largest), encoding='utf-8')
    (root / 'largest-unsupported.json').write_text(json.dumps(largest, indent=2, ensure_ascii=False), encoding='utf-8')
    (root / 'analysis.json').write_text(json.dumps(analysis, indent=2, ensure_ascii=False), encoding='utf-8')
    print(root / 'report.md')


if __name__ == '__main__':
    report(Path(sys.argv[1]))
