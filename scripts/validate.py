#!/usr/bin/env python3
"""One command for repository checks; optional disposable PostgreSQL and performance matrix."""
import argparse
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--postgres', action='store_true')
    parser.add_argument('--performance', action='store_true')
    parser.add_argument('--baseline', default='v0.7.13')
    parser.add_argument('--large', action='store_true')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    output = (args.output or Path(tempfile.mkdtemp(prefix='sheetforge-validation-'))).resolve()
    output.mkdir(parents=True, exist_ok=True)
    results = []
    def run(name, command, env=None):
        print(f'RUN {name}', flush=True)
        started = time.monotonic()
        log = output / f'{name}.log'
        with log.open('w') as stream:
            result = subprocess.run(command, cwd=repo, env=env, stdout=stream, stderr=stream)
        results.append(dict(name=name, seconds=round(time.monotonic()-started, 3), exit_code=result.returncode, log=str(log)))
        (output/'validation.json').write_text(json.dumps(results, ensure_ascii=False, indent=2))
        print(f'{"PASS" if result.returncode == 0 else "FAIL"} {name} ({results[-1]["seconds"]}s)', flush=True)
        if result.returncode:
            print('\n'.join(log.read_text(errors='replace').splitlines()[-65:]), flush=True)
            raise SystemExit(result.returncode)
    manifest = ['--manifest-path', 'src-tauri/Cargo.toml']
    run('format', ['cargo','fmt',*manifest,'--all','--','--check'])
    run('clippy', ['cargo','clippy',*manifest,'--locked','--all-targets','--all-features','--','-D','warnings'])
    run('rust', ['cargo','test',*manifest,'--locked','--quiet'])
    run('audit', ['cargo','audit','--file','src-tauri/Cargo.lock'])
    run('frontend-tests', ['npm','test'])
    run('frontend-build', ['npm','run','build'])
    if args.postgres:
        container = f'sheetforge-test-{secrets.token_hex(5)}'
        password = secrets.token_urlsafe(24)
        env = dict(os.environ, POSTGRES_PASSWORD=password)
        try:
            run('postgres-start', ['docker','run','--rm','--detach','--name',container,'--memory=2g','--cpus=2',
                '-e','POSTGRES_PASSWORD','-e','POSTGRES_DB=sheetforge_dt','-p','127.0.0.1::5432','postgres:17'], env)
            for _ in range(60):
                if subprocess.run(['docker','exec',container,'pg_isready','-U','postgres'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
                    break
                time.sleep(0.5)
            else:
                raise RuntimeError('PostgreSQL did not start')
            port = subprocess.check_output(['docker','port',container,'5432'], text=True).strip().rsplit(':',1)[1]
            env = dict(os.environ, SHEETFORGE_TEST_POSTGRES_PORT=port,
                SHEETFORGE_TEST_POSTGRES_DATABASE='sheetforge_dt', SHEETFORGE_TEST_POSTGRES_USER='postgres',
                SHEETFORGE_TEST_POSTGRES_PASSWORD=password)
            run('postgres-integration', ['cargo','test',*manifest,'--locked','postgres_integration','--','--ignored'], env)
        finally:
            subprocess.run(['docker','rm','--force',container], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if args.performance:
        run('frontend-performance', [sys.executable, str(repo/'scripts/performance_frontend.py'), str(output/'frontend-performance'), '--baseline', args.baseline])
        command = [sys.executable, str(repo/'scripts/performance_matrix.py'), str(output/'performance'), '--baseline', args.baseline]
        if args.large: command.append('--large')
        run('performance', command)
        run('performance-report', [sys.executable, str(repo/'scripts/performance_report.py'), str(output/'performance'), '--frontend', str(output/'frontend-performance')])
    print(f'Results: {output / "validation.json"}', flush=True)


if __name__ == '__main__':
    main()
