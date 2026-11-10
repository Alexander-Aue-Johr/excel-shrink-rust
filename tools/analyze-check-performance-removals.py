"""Explain completed Check Performance size wins using independent controls."""
import argparse
import collections
import csv
import hashlib
import json
from pathlib import Path
import posixpath
import shutil
import subprocess
import xml.etree.ElementTree as ET
import zipfile


def small_xml(archive, name):
    if archive.getinfo(name).file_size > 8 * 1024 * 1024:
        raise ValueError("Small-part XML limit exceeded for " + name)
    return ET.fromstring(archive.read(name))


def canonical(node):
    return [node.tag, sorted(node.attrib.items()), node.text if node.text and node.text.strip() else None,
            [canonical(child) for child in node]]


def style_inventory(archive):
    root = small_xml(archive, 'xl/styles.xml')
    def table(name):
        node = root.find('{*}' + name)
        return list(node) if node is not None else []
    tables = {name: table(name) for name in ('fonts', 'fills', 'borders', 'cellStyleXfs')}
    formats = {int(node.get('numFmtId')): node.get('formatCode') for node in table('numFmts')}
    def resolved(xf, include_base):
        result = {'attributes': {k: v for k, v in xf.attrib.items()
                                 if k not in ('fontId', 'fillId', 'borderId', 'numFmtId', 'xfId')},
                  'children': [canonical(child) for child in xf]}
        for attribute, name in (('fontId', 'fonts'), ('fillId', 'fills'), ('borderId', 'borders')):
            index = int(xf.get(attribute, '0'))
            result[name] = canonical(tables[name][index]) if index < len(tables[name]) else ['unresolved', index]
        number = int(xf.get('numFmtId', '0'))
        result['number_format'] = formats.get(number, 'built-in ' + str(number))
        if include_base:
            index = int(xf.get('xfId', '0'))
            result['base'] = resolved(tables['cellStyleXfs'][index], False) if index < len(tables['cellStyleXfs']) else None
        return result
    signatures, details = [], []
    for index, xf in enumerate(table('cellXfs')):
        structure = resolved(xf, True)
        signatures.append(hashlib.sha256(json.dumps(structure, sort_keys=True).encode()).hexdigest())
        font = tables['fonts'][int(xf.get('fontId', '0'))]
        fill = tables['fills'][int(xf.get('fillId', '0'))]
        border = tables['borders'][int(xf.get('borderId', '0'))]
        def font_value(name):
            node = font.find('{*}' + name)
            return node.get('val') if node is not None else None
        pattern = fill.find('{*}patternFill')
        color = pattern.find('{*}fgColor') if pattern is not None else None
        alignment = xf.find('{*}alignment')
        details.append({'style': index, 'xf': dict(xf.attrib),
                        'font': {'name': font_value('name'), 'size': font_value('sz'),
                                 'bold': font.find('{*}b') is not None, 'italic': font.find('{*}i') is not None},
                        'fill': {'pattern': pattern.get('patternType') if pattern is not None else None,
                                 'color': dict(color.attrib) if color is not None else {}},
                        'borders': {side.tag.rsplit('}', 1)[-1]: side.get('style') for side in border if side.get('style')},
                        'alignment': dict(alignment.attrib) if alignment is not None else {},
                        'number_format': structure['number_format'], 'resolved': structure})
    return signatures, details


def variant(path):
    with zipfile.ZipFile(path) as archive:
        workbook = small_xml(archive, 'xl/workbook.xml')
        relationships = {node.get('Id'): node.get('Target') for node in small_xml(archive, 'xl/_rels/workbook.xml.rels')}
        sheets = {}
        unlinked_sheets = []
        for node in workbook.find('{*}sheets'):
            relationship = next(value for key, value in node.attrib.items() if key.endswith('}id'))
            if not relationship:
                unlinked_sheets.append(dict(node.attrib))
                continue
            target = relationships[relationship]
            part = target.lstrip('/') if target.startswith('/') else posixpath.normpath('xl/' + target)
            if part.startswith('xl/worksheets/'):
                sheets[node.get('name')] = part
        styles, details = style_inventory(archive)
        parts = {info.filename: {'bytes': info.file_size, 'compressed_bytes': info.compress_size, 'crc32': info.CRC}
                 for info in archive.infolist()}
    return {'path': str(path), 'sheets': sheets, 'styles': styles, 'style_details': details, 'parts': parts,
            'unlinked_sheets': unlinked_sheets}


def run(comparison, output, probe):
    output.mkdir(parents=True, exist_ok=False)
    shutil.copy2(comparison, output / 'frozen-comparison.csv')
    with comparison.open(encoding='utf-8-sig', newline='') as stream:
        winners = [row for row in csv.DictReader(stream) if row['smaller_result'] == 'check_performance']
    cases = []
    for row in winners:
        attempt = Path(row['check_performance_path']).parents[1]
        paths = {'default': Path(row['default_path']), 'cp_after': Path(row['check_performance_path']),
                 'original': attempt / 'inputs/original.xlsx',
                 'saved_original': attempt / 'artifacts/original.saved.xlsx',
                 'cp_before': attempt / 'artifacts/check-performance.before-optimization.xlsx'}
        cases.append({'id': row['case_id'], 'source_member': row['source_member'],
                      'file_advantage_bytes': int(row['default_xlsx_bytes']) - int(row['cp_after_xlsx_bytes']),
                      'variants': {name: variant(path) for name, path in paths.items()}})
    manifest = output / 'manifest.json'
    probe_cases = [{'id': case['id'], 'variants': {name: {key: value[key] for key in ('path', 'sheets', 'styles')}
                    for name, value in case['variants'].items()}} for case in cases]
    manifest.write_text(json.dumps(probe_cases, indent=2) + '\n', encoding='utf8')
    subprocess.run([str(probe), str(manifest), str(output / 'sheets')], check=True)
    summaries, details, part_rows = [], [], []
    for case in cases:
        analysis = json.loads((output / 'sheets' / (case['id'] + '.json')).read_bytes())
        counts = collections.Counter()
        groups = collections.Counter()
        row_groups = collections.Counter()
        errors = []
        for sheet in analysis['sheets']:
            if 'error' in sheet:
                errors.append(sheet)
                continue
            for key in ('cells_only_in_default', 'default_only_cells_with_payload_elements',
                        'empty_rows_only_in_default', 'default_only_cells_below_last_payload_row'):
                counts[key] += sheet[key]
            groups.update(sheet['cell_attribution'])
            row_groups.update(sheet['empty_row_attribution'])
            style_details = case['variants']['default']['style_details']
            sheet['dominant_extra_cell_styles'] = [
                {'count': count, **style_details[int(style)]}
                for style, count in sorted(sheet['default_extra_cell_styles'].items(), key=lambda x: x[1], reverse=True)[:5]]
        names = set().union(*(v['parts'] for v in case['variants'].values()))
        for name in sorted(names):
            parts = {key: value['parts'].get(name) for key, value in case['variants'].items()}
            d, a = parts['default'], parts['cp_after']
            if d == a:
                continue
            record = {'case_id': case['id'], 'part': name}
            for key, info in parts.items():
                record[key + '_bytes'] = info['bytes'] if info else None
                record[key + '_compressed_bytes'] = info['compressed_bytes'] if info else None
            record['uncompressed_advantage'] = (d['bytes'] if d else 0) - (a['bytes'] if a else 0)
            record['compressed_advantage'] = (d['compressed_bytes'] if d else 0) - (a['compressed_bytes'] if a else 0)
            part_rows.append(record)
        row = {'case_id': case['id'], 'source_member': case['source_member'],
               'file_advantage_bytes': case['file_advantage_bytes'], **counts,
               'cells_removed_during_cp': groups['removed_during_check_performance'],
               'cells_absent_after_original_open_save': groups['absent_after_original_open_save'],
               'cells_absent_after_cp_copy_open_save': groups['absent_after_cp_copy_open_save'],
               'cells_absent_from_raw_original': groups['absent_from_raw_original'],
               'empty_rows_removed_during_cp': row_groups['removed_during_check_performance'],
               'empty_rows_absent_before_cp': sum(v for k, v in row_groups.items() if k != 'removed_during_check_performance'),
               'sheet_errors': len(errors)}
        summaries.append(row)
        details.append({'id': case['id'], 'source_member': case['source_member'], **analysis,
                        'unlinked_sheets': {name: value['unlinked_sheets']
                                           for name, value in case['variants'].items()
                                           if value['unlinked_sheets']}})
    for name, rows in [('summary.csv', summaries), ('parts.csv', part_rows)]:
        with (output / name).open('w', encoding='utf-8-sig', newline='') as stream:
            writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)
    (output / 'details.json').write_text(json.dumps(details, indent=2) + '\n', encoding='utf8')
    print(json.dumps({'cases': len(cases), 'totals': dict(sum((collections.Counter({
        key: value for key, value in row.items() if isinstance(value, int)}) for row in summaries), collections.Counter()))}), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--comparison', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--probe', type=Path, required=True)
    args = parser.parse_args()
    run(args.comparison, args.output, args.probe)
