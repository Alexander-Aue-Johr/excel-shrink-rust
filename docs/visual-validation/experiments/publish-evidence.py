#!/usr/bin/env python3
"""Publish allowlisted evidence from one completed managed evaluation run."""
from pathlib import Path
import argparse,sys
if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run-directory',type=Path,required=True)
    parser.add_argument('--repo-root',type=Path,default=Path(__file__).resolve().parents[3])
    args=parser.parse_args()
    sys.path.insert(0,str(Path(__file__).resolve().parent/'_shared/suite'))
    from publication import publish
    print(publish(args.repo_root.resolve(),args.run_directory.resolve()))
