"""Build, shrink and optionally compare identical Excel ranges via VBA."""
import argparse, csv, hashlib, json, os, subprocess, sys, time, uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
BASIC = ['--remove-textless-default-font-representatives', '--remove-textless-style-representatives']
PROFILES = {
    'default': [],
    'remove-basic-textless-representatives': BASIC,
    'remove-all-textless-representatives': BASIC + ['--remove-textless-column-style-representatives'],
    'remove-unreferenced-whitespace-cells': ['--remove-unreferenced-whitespace-cells'],
    'keep-unreferenced-hidden-content-cells': ['--keep-unreferenced-hidden-content-cells'],
    'keep-inactive-protection-cells-in-data-range': ['--inactive-protection-cell-policy', 'keep-in-data-range'],
    'keep-all-inactive-protection-cells': ['--inactive-protection-cell-policy', 'keep-all'],
    'maximum-cleaning': BASIC + ['--remove-textless-column-style-representatives', '--remove-unreferenced-whitespace-cells'],
}

def windows_path(path):
    if os.name == 'nt':
        return str(path)
    return subprocess.check_output(['wslpath', '-w', str(path)], text=True).strip()

def linux_context():
    if os.name != 'nt':
        return [], str(ROOT)
    parts = str(ROOT).split('\\')
    if len(parts) < 5 or parts[2].lower() not in ('wsl.localhost', 'wsl$'):
        raise RuntimeError('Windows: open the WSL repository via its UNC path, or run this task in WSL.')
    return ['wsl.exe', '-d', parts[3], '--cd', '/' + '/'.join(parts[4:]), '--exec'], '/' + '/'.join(parts[4:])

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--profile', choices=[*PROFILES, 'all'], default='default')
    ap.add_argument('--height', action='store_true')
    ap.add_argument('--resave-originals', action='store_true')
    ap.add_argument('--only-file', default='', help='Exact corpus filename, or * for all')
    args = ap.parse_args()
    if args.resave_originals and not args.height:
        ap.error('--resave-originals requires --height')
    corpus = ROOT / 'corpus-local/original_destatis_xlsx_files'
    files = sorted(p for p in corpus.glob('*.xlsx') if not p.name.startswith('~$')
                   and (args.only_file in ('', '*') or p.name == args.only_file))
    if not files:
        ap.error('No matching corpus XLSX files')
    prefix, linux_root = linux_context()
    # Explicit executable, no shell interpolation; works with spaces and umlauts.
    cargo = str(Path.home() / '.cargo/bin/cargo') if os.name != 'nt' else 'cargo'
    if os.name == 'nt':
        cargo = subprocess.check_output(prefix + ['printenv', 'HOME'], text=True).strip() + '/.cargo/bin/cargo'
    subprocess.run(prefix + [cargo, 'build', '--release', '--locked', '-p', 'excel-shrink-cli'], cwd=ROOT, check=True)
    run = ROOT / '.tmp/height-checks' / (time.strftime('%Y%m%d-%H%M%S') + '-' + uuid.uuid4().hex[:8])
    run.mkdir(parents=True)
    linux_run = linux_root + '/' + run.relative_to(ROOT).as_posix()
    selected = PROFILES if args.profile == 'all' else {args.profile: PROFILES[args.profile]}
    manifest = {'reference': 'excel-resaved-copy' if args.resave_originals else 'original-copy',
                'resave_originals': args.resave_originals, 'files': [], 'profiles': selected,
                'timings': [], 'helper_sha256': hashlib.sha256((HERE/'HeightProbeHost.xlsm').read_bytes()).hexdigest()}
    for p in files:
        manifest['files'].append({'name': p.name, 'source': windows_path(p),
            'sha256': hashlib.sha256(p.read_bytes()).hexdigest()})
    manifest['profile_directories'] = {p: f'p{i:02d}' for i, p in enumerate(selected)}
    for profile, flags in selected.items():
        folder = manifest['profile_directories'][profile]
        dest = run / folder / 'shrunk'; dest.mkdir(parents=True)
        start = time.perf_counter()
        with (run / folder / 'shrink.log').open('wb') as log:
            for p in files:
                print(f'Shrink {profile}: {p.name}', flush=True)
                subprocess.run(prefix + [linux_root + '/target/release/excel-shrink-cli',
                    linux_root + '/' + p.relative_to(ROOT).as_posix(), linux_run + '/' + folder + '/shrunk', *flags],
                    cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
        manifest['timings'].append({'profile': profile, 'shrink_seconds': time.perf_counter()-start})
    (run/'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding='utf-8')
    print(f'Results: {run}', flush=True)
    if not args.height:
        return 0
    subprocess.run(['powershell.exe', '-NoProfile', '-STA', '-ExecutionPolicy', 'Bypass',
                    '-File', windows_path(HERE/'measure.ps1'), '-RunRoot', windows_path(run)], check=True)
    rows = json.loads((run/'comparison.json').read_text(encoding='utf-8-sig'))
    expected = len(selected) * sum(f['sheet_count'] for f in json.loads((run/'measured-files.json').read_text(encoding='utf-8-sig')))
    if len(rows) != expected:
        raise RuntimeError('Incomplete measurement')
    changed = [r for r in rows if abs(r['delta_points']) > 1e-7]
    with (run/'differences.csv').open('w', encoding='utf-8-sig', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]), delimiter=';'); w.writeheader(); w.writerows(changed)
    lines = ['# Identical-range height comparison', '', f'Reference: {manifest["reference"]}.', '',
        'The shrunk UsedRange address is measured in both files. Points × 4/3 gives pixel equivalents at 96 DPI / 100%. Total heights can hide compensating changes in individual rows. No screenshot comparison.', '',
        '| Profile | Sheets | Changed | Smaller | Larger |', '|---|---:|---:|---:|---:|']
    for p in selected:
        subset = [r for r in rows if r['profile'] == p]
        delta = [r for r in changed if r['profile'] == p]
        lines.append(f'| {p} | {len(subset)} | {len(delta)} | {sum(r["delta_points"]<0 for r in delta)} | {sum(r["delta_points"]>0 for r in delta)} |')
    lines += ['', 'Details: differences.csv; all measurements: comparison.json; configuration and timings: manifest.json.']
    (run/'report.md').write_text('\n'.join(lines)+'\n', encoding='utf-8')
    print('\n'.join(lines), flush=True)
    return 2 if changed else 0

if __name__ == '__main__':
    sys.exit(main())
