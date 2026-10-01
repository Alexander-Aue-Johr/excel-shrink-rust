#!/usr/bin/env python3
"""Run the actual native old/view comparator benchmark, not a Python imitation."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

def main() -> int:
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[2])
    parser.add_argument('--left',type=Path)
    parser.add_argument('--right',type=Path)
    parser.add_argument('--rows',type=int,default=20000)
    parser.add_argument('--allocations',action='store_true',help='Measure allocation counts; adds instrumentation overhead to timings.')
    parser.add_argument('--output',type=Path,default=Path('value-benchmark-v9.json'))
    args=parser.parse_args()
    if bool(args.left)!=bool(args.right):parser.error('--left and --right must be supplied together')
    if not 1<=args.rows<=1048576:parser.error('--rows must be between 1 and 1048576')
    cargo=shutil.which('cargo')
    if cargo is None:parser.error('cargo is not on PATH. Run this in your Rust/WSL development environment.')
    root=args.root.resolve()
    if not (root/'excel-shrink-cli/src/regression/value_views.rs').is_file():parser.error('The v9 patch is not installed in --root.')
    env=os.environ.copy()
    env['EXCEL_SHRINK_BENCH_ROWS']=str(args.rows)
    env['EXCEL_SHRINK_BENCH_ALLOCATIONS']='1' if args.allocations else '0'
    for name,path in [('LEFT',args.left),('RIGHT',args.right)]:
        key='EXCEL_SHRINK_BENCH_'+name
        if path is not None:
            path=path.resolve()
            if not path.is_file():parser.error(f'Workbook not found: {path}')
            env[key]=str(path)
        else:env.pop(key,None)
    command=[cargo,'test','-p','excel-shrink-cli','--release','v9_benchmark_value_comparison','--','--ignored','--nocapture','--test-threads=1']
    print('Running:',subprocess.list2cmdline(command),flush=True)
    process=subprocess.Popen(command,cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,encoding='utf-8',errors='replace')
    report=None
    assert process.stdout is not None
    for line in process.stdout:
        print(line,end='')
        marker='V9_VALUE_BENCHMARK '
        if marker in line:report=json.loads(line.split(marker,1)[1])
    code=process.wait()
    if code:return code
    if report is None:
        print('Native test completed without a benchmark record.',file=sys.stderr);return 1
    report['command']=command
    output=args.output.resolve();output.parent.mkdir(parents=True,exist_ok=True)
    output.write_text(json.dumps(report,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
    print(f'Benchmark written to {output}')
    return 0
if __name__=='__main__':
    raise SystemExit(main())
