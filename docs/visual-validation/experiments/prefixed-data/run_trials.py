"""Measure actual shrink potential after verified namespace-only normalization of retained cases."""
import argparse
import hashlib
import json
import subprocess
import zipfile
from pathlib import Path
from normalize import normalize, XmlCopy


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def shrink(binary, source, output, report, log):
    with log.open('wb') as stream:
        process = subprocess.run(
            [str(binary), str(source), str(output), '--disable-multiprocessing',
             '--rayon-threads', '2', '--batch-report', str(report)],
            stdout=stream, stderr=subprocess.STDOUT,
        )
    outcome = json.loads(report.read_text(encoding='utf-8'))
    if process.returncode or outcome['summary']['failed']:
        raise RuntimeError(f'Shrink failed; see {log}')
    return outcome['files'][0]


def normalize_rust(binary, source, target, report, log):
    with log.open('wb') as stream:
        process = subprocess.run(
            [str(binary), 'regression', 'prefix-normalize', '--input', str(source),
             '--output', str(target), '--report', str(report)],
            stdout=stream, stderr=subprocess.STDOUT,
        )
    if process.returncode:
        raise RuntimeError(f'Rust normalization failed; see {log}')
    return json.loads(report.read_text(encoding='utf-8'))


def trial(record, inventory, output, binary, rust_normalize):
    source = inventory / 'sources' / f"{record['source_id']:05}.xlsx"
    identity = digest(source)
    case = output / f"{record['source_id']:05}"
    case.mkdir()
    result = {
        'source_id': record['source_id'], 'workbook': record['workbook'],
        'source_sha256': identity, 'original_bytes': source.stat().st_size,
    }
    raw = shrink(binary, source, case / 'raw-output', case / 'raw-report.json', case / 'raw.log')
    result['original_shrinker_outcome'] = raw
    if raw['status'] != 'skipped':
        raise RuntimeError('Expected the existing namespace policy to skip the original')
    normalized = case / 'normalized' / source.name
    if rust_normalize:
        evidence = normalize_rust(binary, source, normalized, case / 'rust-normalization.json', case / 'normalize.log')
    else:
        evidence = normalize(source, normalized)
    result['normalizer'] = 'rust-expanded-name-stream' if rust_normalize else 'python-expat'
    (case / 'normalization.json').write_text(json.dumps(evidence, indent=2), encoding='utf-8')
    optimized = shrink(binary, normalized, case / 'shrunk', case / 'shrink-report.json', case / 'shrink.log')
    if optimized['status'] not in ('processed', 'preserved'):
        raise RuntimeError('Normalized trial was skipped; it cannot establish cleanup potential')
    target = case / 'shrunk' / source.name
    result.update(normalized_bytes=normalized.stat().st_size, shrunk_bytes=target.stat().st_size,
                  normalized_sha256=digest(normalized), shrunk_sha256=digest(target))
    result['byte_identical_after_shrink'] = result['normalized_sha256'] == result['shrunk_sha256']
    result['shrink_saved_bytes'] = result['normalized_bytes'] - result['shrunk_bytes']
    result['max_row'] = max((value['max_row'] for value in evidence.values()), default=0)
    result['cells'] = sum(value['cells'] for value in evidence.values())
    result['changed_xml'] = []
    if not result['byte_identical_after_shrink']:
        after_evidence = None
        if rust_normalize:
            after_evidence = normalize_rust(
                binary, target, case / 'verified-shrunk.xlsx',
                case / 'shrunk-xml.json', case / 'verify.log',
            )
        with zipfile.ZipFile(normalized) as before, zipfile.ZipFile(target) as after:
            for name in sorted(set(before.namelist()) | set(after.namelist())):
                if not name.endswith('.xml'):
                    continue
                if name not in before.namelist() or name not in after.namelist():
                    result['changed_xml'].append({'part': name, 'entry_added_or_removed': True})
                    continue
                with before.open(name) as a, after.open(name) as b:
                    different = hashlib.file_digest(a, 'sha256').digest() != hashlib.file_digest(b, 'sha256').digest()
                if different:
                    if after_evidence is not None and name in after_evidence:
                        actual = after_evidence[name]
                    else:
                        with after.open(name) as entry:
                            actual = XmlCopy().read(entry)
                    result['changed_xml'].append({
                        'part': name, 'before_bytes': before.getinfo(name).file_size,
                        'after_bytes': after.getinfo(name).file_size,
                        'after_xml': actual, 'before_xml': evidence.get(name),
                    })
    if digest(source) != identity:
        raise RuntimeError('Original source changed during the trial')
    result['status'] = 'completed'
    return result


def run(args):
    args.output.mkdir(parents=True, exist_ok=True)
    inventory = (args.inventory / 'workbooks.jsonl').read_text(encoding='utf-8')
    records = [json.loads(line) for line in inventory.splitlines()]
    records = sorted(
        (record for record in records if record['hits']),
        key=lambda record: (record['input_bytes'], record['source_id']),
    )
    for record in records:
        report = args.output / f"{record['source_id']:05}.json"
        if report.exists():
            continue
        try:
            result = trial(record, args.inventory, args.output, args.binary.resolve(), args.rust_normalize)
        except Exception as error:
            result = {
                'source_id': record['source_id'], 'workbook': record['workbook'],
                'status': 'error', 'error': str(error),
            }
        report.write_text(json.dumps(result, indent=2), encoding='utf-8')
        print(json.dumps(result), flush=True)
    results = [
        json.loads(path.read_text(encoding='utf-8'))
        for path in args.output.glob('*.json') if path.name != 'summary.json'
    ]
    summary = {
        'matched_in_inventory_snapshot': len(records), 'trials': len(results),
        'completed': sum(r['status'] == 'completed' for r in results),
        'errors': sum(r['status'] == 'error' for r in results),
        'byte_identical_after_shrink': sum(r.get('byte_identical_after_shrink', False) for r in results),
        'with_xml_changes': sum(bool(r.get('changed_xml')) for r in results),
    }
    summary['positive_size_savings'] = sum(r.get('shrink_saved_bytes', 0) > 0 for r in results)
    summary['changed_expanded_xml'] = sum(
        any(part.get('after_xml') != part.get('before_xml') for part in result.get('changed_xml', []))
        for result in results
    )
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2), encoding='utf-8')
    print(json.dumps(summary), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inventory', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--rust-normalize', action='store_true')
    run(parser.parse_args())
