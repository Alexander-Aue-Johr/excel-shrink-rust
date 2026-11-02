"""Inspect retained pivot CF metadata without opening Excel or scanning worksheet cell data."""
import argparse
import hashlib
import json
import posixpath
import zipfile
import xml.etree.ElementTree as ET
from pathlib import Path

NS = {'m': 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'}


def inspect(inventory):
    cases = []
    for line in (inventory / 'sheets.jsonl').open(encoding='utf-8'):
        sheet = json.loads(line)
        if sheet['source_id'] not in (4204, 6160):
            continue
        blocks = [block for block in sheet['blocks']
                  if block['attributes'].get('pivot') == '1'
                  and all(rule['attributes'].get('type') == 'cellIs' for rule in block['rules'])]
        if not blocks:
            continue
        source = inventory / 'sources' / ('%05d.xlsx' % sheet['source_id'])
        with zipfile.ZipFile(source) as archive:
            part = sheet['part']
            rels = posixpath.join(posixpath.dirname(part), '_rels', posixpath.basename(part) + '.rels')
            pivots = []
            for relationship in ET.fromstring(archive.read(rels)):
                if not relationship.attrib.get('Type', '').endswith('/pivotTable'):
                    continue
                if relationship.attrib.get('TargetMode') == 'External':
                    raise ValueError('External pivot relationship is not supported by this inspection')
                target = relationship.attrib['Target']
                target = target.lstrip('/') if target.startswith('/') else posixpath.normpath(posixpath.join(posixpath.dirname(part), target))
                raw = archive.read(target)
                pivot = ET.fromstring(raw)
                location = pivot.find('m:location', NS)
                pivots.append((target, pivot, location.attrib if location is not None else None, hashlib.sha256(raw).hexdigest()))
            for block in blocks:
                matches = []
                for rule in block['rules']:
                    priority = rule['attributes']['priority']
                    for target, pivot, location, digest in pivots:
                        for formatting in pivot.findall('m:conditionalFormats/m:conditionalFormat', NS):
                            if formatting.attrib.get('priority') == priority:
                                matches.append({
                                    'pivot_part': target, 'sha256': digest, 'location': location,
                                    'pivot_attributes': pivot.attrib,
                                    'conditional_format': formatting.attrib,
                                    'pivot_areas_xml': [ET.tostring(area, encoding='unicode') for area in formatting.findall('m:pivotAreas', NS)],
                                })
                if len(matches) != len(block['rules']):
                    raise ValueError('Pivot priority mapping is not unique and complete')
                cases.append({
                    'workbook': sheet['workbook'], 'source_id': sheet['source_id'],
                    'worksheet_part': part, 'sqref': block['sqref'],
                    'rules': [rule['attributes'] | {'formulas': rule['formulas']} for rule in block['rules']],
                    'pivot_metadata': matches,
                })
    if len(cases) != 8:
        raise ValueError(f'Expected eight retained pivot cellIs blocks, found {len(cases)}')
    return {
        'decision': 'Retain capture fallback; pivot-area dependencies are not modeled',
        'scope': 'Metadata investigation, not a visual invariance proof',
        'cases': cases,
    }


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('inventory', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    result = inspect(args.inventory)
    with args.output.open('x', encoding='utf-8') as output:
        json.dump(result, output, indent=2)
