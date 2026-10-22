#!/usr/bin/env python3
"""Batch Node metadata benchmarks and prepare paired production-built browser fixtures."""
import argparse
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

REPO = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output',type=Path)
    parser.add_argument('--baseline',default='v0.7.13')
    args=parser.parse_args(); base=args.output.resolve(); base.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='sheetforge-ui-baseline-') as temp:
        old=Path(temp)
        archive=subprocess.check_output(['git','archive',args.baseline],cwd=REPO)
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar: tar.extractall(old,filter='data')
        shutil.copytree(REPO/'scripts/performance',old/'scripts/performance')
        (old/'node_modules').symlink_to(REPO/'node_modules',target_is_directory=True)
        for label, checkout in [('baseline',old),('current',REPO)]:
            env=dict(os.environ,SF_FRONTEND_RESULTS=str(base/f'{label}-metadata.json'),SF_UI_OUTPUT=str(base/f'{label}-ui'))
            for name,cmd in [('metadata',['npx','vitest','run','--config','scripts/performance/vitest.config.ts']),('ui-build',['npx','vite','build','--config','scripts/performance/ui.config.ts'])]:
                with (base/f'{label}-{name}.log').open('w') as log:
                    result=subprocess.run(cmd,cwd=checkout,env=env,stdout=log,stderr=log)
                if result.returncode: raise RuntimeError(f'{label} {name}: {base}/{label}-{name}.log')
                print(f'PASS {label} {name}',flush=True)
    print(f'Browser fixtures: {base}/baseline-ui/scripts/performance/index.html and current-ui/scripts/performance/index.html',flush=True)


if __name__=='__main__':main()
