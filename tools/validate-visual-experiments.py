#!/usr/bin/env python3
"""Portable validation for the capture suite; never starts Excel."""
import json,subprocess,sys
from pathlib import Path
root=Path(__file__).resolve().parents[1];exp=root/'docs/visual-validation/experiments'
commands=[[sys.executable,'-m','unittest','discover','-s',str(exp/'tests'),'-p','test_*.py'],
 [sys.executable,'-m','unittest','discover','-s',str(exp/'range-image-calibration/tests'),'-p','test_*.py'],
 ['node','--test']+[str(p) for folder in (exp/'tests',exp/'range-image-calibration/tests') for p in sorted(folder.glob('*.test.cjs'))]]
for cmd in commands:
    try:r=subprocess.run(cmd,cwd=root,check=False)
    except FileNotFoundError as e:print(str(e),file=sys.stderr);sys.exit(1)
    if r.returncode:sys.exit(r.returncode)
print('Portable checks passed. Windows/Excel execution is a separate check.')
