#!/usr/bin/env python3
"""Paired release-profile matrix, shared synthetic files, three runs and semantic hashes.

Builds historical source in an isolated temporary directory, never edits that checkout's
Git state. Stage logs and an incremental JSON report survive failures; reruns resume only
when fixture hashes and source/build fingerprints still agree.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tarfile
import tempfile
import time
import zipfile

REPO = Path(__file__).resolve().parents[1]
TEST = 'database::performance_matrix::run_stage'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def xml_hashes(path):
    result = {}
    with zipfile.ZipFile(path) as archive:
        for name in sorted(archive.namelist()):
            if name.startswith('xl/worksheets/') or name in ('xl/styles.xml','xl/sharedStrings.xml','xl/workbook.xml'):
                with archive.open(name) as stream: result[name] = hashlib.file_digest(stream,'sha256').hexdigest()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--baseline', default='v0.7.13')
    parser.add_argument('--rerun-stages', nargs='*', default=[], help='remeasure only case:stage pairs with existing identical files')
    parser.add_argument('--rerun-cases', nargs='*', default=[], help='rerun measurements, keep the identical generated files')
    parser.add_argument('--large', action='store_true', help='also verify ten million rows once (no timing claim)')
    args = parser.parse_args()
    base = args.output.resolve()
    base.mkdir(parents=True, exist_ok=True)
    (base/'logs').mkdir(exist_ok=True)
    cases = json.loads((REPO/'scripts/performance/cases.json').read_text())
    cases = [c for c in cases if args.large or c['name'] != 'csv_10m_8']
    (base/'cases.json').write_text(json.dumps(cases, indent=2))
    baseline_sha = subprocess.check_output(['git','rev-parse',args.baseline], cwd=REPO, text=True).strip()
    source = hashlib.sha256()
    for path in sorted((REPO/'src-tauri/src').rglob('*.rs')):
        source.update(str(path.relative_to(REPO)).encode()); source.update(path.read_bytes())
    source.update((REPO/'src-tauri/Cargo.lock').read_bytes())
    source.update((REPO/'src-tauri/Cargo.toml').read_bytes())
    fingerprint = dict(baseline=baseline_sha, current_source=source.hexdigest(),
        rustc=subprocess.check_output(['rustc','--version'], text=True).strip(),
        profile='release opt-level=z lto=fat codegen-units=1', platform=platform.platform(), cases=cases)
    report_file = base/'matrix.json'
    report = json.loads(report_file.read_text()) if report_file.exists() else dict(metadata=fingerprint, runs=[], fixtures={}, outputs={})
    assert report['metadata'] == fingerprint, 'Source/toolchain/cases changed: use a new output directory.'
    rerun_stages = {tuple(value.split(':',1)) for value in args.rerun_stages}
    if rerun_stages:
        known = {(r['case'],r['stage']) for r in report['runs'] if r['stage'] not in ('generate','stream_hash','verify_output','merge_xlsx_fast')}
        assert rerun_stages <= known, 'Choose an existing measured stage, not generation/verification/fast-export.'
    if rerun_stages:
        report['runs'] = [r for r in report['runs'] if (r['case'],r['stage']) not in rerun_stages]
    if args.rerun_cases:
        report['runs'] = [r for r in report['runs'] if r['case'] not in args.rerun_cases or r['stage'] == 'generate']
    def save(): report_file.write_text(json.dumps(report, ensure_ascii=False, indent=2))
    save()
    env_base = dict(os.environ, SF_REVIEW_DIR=str(base))
    binaries = {}
    def build(checkout, label):
        binary = base/f'engine-{label}'
        if binary.exists() and report.get('built',{}).get(label): return binary
        print(f'BUILD {label}', flush=True)
        command = ['cargo','test','--manifest-path','src-tauri/Cargo.toml','--release','--locked','--no-run','--message-format=json']
        with (base/'logs'/f'build-{label}.log').open('w') as log:
            result = subprocess.run(command, cwd=checkout, text=True, stdout=subprocess.PIPE, stderr=log, env=dict(env_base, CARGO_TARGET_DIR=str(REPO/'src-tauri/target')))
        if result.returncode: raise RuntimeError(f'{label} build failed: {base}/logs/build-{label}.log')
        records = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
        artifact = next(item['executable'] for item in records if item.get('executable') and item.get('profile',{}).get('test'))
        shutil.copy2(artifact, binary)
        report.setdefault('built',{})[label] = True; save()
        return binary
    # Archive baseline sources; only inject the compatible, ignored measurement harness.
    with tempfile.TemporaryDirectory(prefix='sheetforge-perf-baseline-') as temporary:
        checkout = Path(temporary)
        archive = subprocess.check_output(['git','archive',baseline_sha], cwd=REPO)
        import io
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar: tar.extractall(checkout, filter='data')
        shutil.copy2(REPO/'src-tauri/src/database/performance_matrix.rs', checkout/'src-tauri/src/database/performance_matrix.rs')
        with (checkout/'src-tauri/src/database.rs').open('a') as stream: stream.write('\n#[cfg(test)]\nmod performance_matrix;\n')
        binaries['baseline'] = build(checkout,'baseline')
    binaries['current'] = build(REPO,'current')
    def run(case, label, stage, repeat=0):
        key = dict(case=case['name'], label=label, stage=stage, repeat=repeat)
        existing = next((item for item in report['runs'] if all(item.get(k)==v for k,v in key.items())), None)
        if existing: return existing
        log = base/'logs'/f'{case["name"]}-{label}-{stage}-{repeat}.log'
        command = ['/usr/bin/time', '-l' if sys.platform=='darwin' else '-v', str(binaries[label]), '--exact', TEST, '--ignored','--nocapture']
        with log.open('w') as stream:
            result = subprocess.run(command, cwd=REPO, env=dict(env_base,SF_CASE=case['name'],SF_STAGE=stage), stdout=stream,stderr=stream,timeout=900)
        output = log.read_text(errors='replace')
        if result.returncode:
            print('\n'.join(output.splitlines()[-30:]), flush=True)
            raise RuntimeError(f'{key}: failed, see {log}')
        value = json.loads(re.search(r'RESULT (\{[^\n]+\})', output)[1])
        rss_pattern = r'(\d+)\s+maximum resident set size' if sys.platform=='darwin' else r'Maximum resident set size \(kbytes\): (\d+)'
        rss = int(re.search(rss_pattern, output)[1]) * (1 if sys.platform=='darwin' else 1024)
        item = dict(**key, ms=value['ms'], rss_bytes=rss, data=value['data'])
        report['runs'].append(item); save()
        return item
    def equal(case, stage, pairs):
        # Advisory check wording/order may change; data hashes, row/group counts must not.
        for field in ('hash','rows','groups','sheets'):
            values = [item['data'][field] for item in pairs if field in item['data']]
            assert not values or all(value==values[0] for value in values), f'{case["name"]} {stage} changed {field}'
    for case in cases:
        if rerun_stages and not any(name == case['name'] for name,_ in rerun_stages): continue
        name = case['name']; huge = name=='csv_10m_8'
        print(f'CASE {name}', flush=True)
        run(case,'current','generate')
        paths = sorted((base/'fixtures'/name/'sources').glob(f'*.{case["kind"]}'))
        hashes = {p.name: digest(p) for p in paths}
        if name in report['fixtures']: assert report['fixtures'][name] == hashes, 'Fixture changed'
        report['fixtures'][name] = hashes; save()
        stages = ['scan','source_preview_warm','merged_preview_warm','stream_core','merge_xlsx']
        if huge: stages = ['scan','stream_core','merge_xlsx']
        if name in ('csv_1m_16','csv_gbk_100k_64','xlsx_inline_10k_512','xlsx_shared_100k_64'):
            stages += ['stream_filter_late','stream_dedup']
        if name=='csv_100files_1k_64': stages += ['check_pg','check_xlsx']
        for stage in stages:
            if rerun_stages and (name,stage) not in rerun_stages: continue
            pairs = []
            for label in ('baseline','current'):
                for repeat in range(1 if huge else 3): pairs.append(run(case,label,stage,repeat))
                if stage == 'merge_xlsx':
                    # Each revision's final output is read through the typed reader and
                    # hashed against its complete mapped source stream.
                    run(case,label,'stream_hash'); run(case,label,'verify_output')
                    report['outputs'][f'{name}-{label}'] = xml_hashes(base/'fixtures'/name/'output.xlsx'); save()
            equal(case,stage,pairs)
            print(f'  PASS {stage}', flush=True)
        assert report['outputs'][f'{name}-baseline']==report['outputs'][f'{name}-current'], f'{name}: XML changed'
        if name in ('csv_1m_16','xlsx_inline_10k_512','xlsx_numeric_edges_100k_64'):
            normal = report['outputs'][f'{name}-current']
            for repeat in range(3): run(case,'current','merge_xlsx_fast',repeat)
            run(case,'current','verify_output',1)
            assert normal == xml_hashes(base/'fixtures'/name/'output.xlsx'), 'Fast compression changed cells/styles'
        # Cache reuse measured separately from the first preview; setup still affects RSS.
        if not huge and (not rerun_stages or (name,'source_preview_cached_warm') in rerun_stages):
            cached = []
            for label in ('baseline','current'):
                for repeat in range(3): cached.append(run(case,label,'source_preview_cached_warm',repeat))
            equal(case,'source_preview_cached_warm',cached)
        print(f'DONE {name}', flush=True)
    summaries = []
    for case in cases:
        for stage in sorted({r['stage'] for r in report['runs'] if r['case']==case['name']}):
            groups = {label:[r for r in report['runs'] if r['case']==case['name'] and r['stage']==stage and r['label']==label] for label in ('baseline','current')}
            if not all(len(values)==3 for values in groups.values()): continue
            item = dict(case=case['name'], stage=stage)
            for label, values in groups.items():
                item[label] = dict(ms=statistics.median(v['ms'] for v in values), rss_bytes=statistics.median(v['rss_bytes'] for v in values))
            item['time_change'] = item['current']['ms']/item['baseline']['ms']-1
            item['rss_change'] = item['current']['rss_bytes']/item['baseline']['rss_bytes']-1
            item['over_10_percent'] = {metric:item[f'{metric}_change']>0.10 for metric in ('time','rss')}
            summaries.append(item)
    report['summary']=summaries
    required = []
    for case in cases:
        name = case['name']; huge = name == 'csv_10m_8'
        stages = ['scan','stream_core','merge_xlsx'] if huge else ['scan','source_preview_warm','merged_preview_warm','stream_core','merge_xlsx','source_preview_cached_warm']
        if name in ('csv_1m_16','csv_gbk_100k_64','xlsx_inline_10k_512','xlsx_shared_100k_64'): stages += ['stream_filter_late','stream_dedup']
        if name == 'csv_100files_1k_64': stages += ['check_pg','check_xlsx']
        for label in ('baseline','current'):
            for stage in stages:
                required += [(name,label,stage,repeat) for repeat in range(1 if huge else 3)]
            required += [(name,label,stage,0) for stage in ('stream_hash','verify_output')]
        if name in ('csv_1m_16','xlsx_inline_10k_512','xlsx_numeric_edges_100k_64'):
            required += [(name,'current','merge_xlsx_fast',repeat) for repeat in range(3)]
            required += [(name,'current','verify_output',1)]
    found = {(r['case'],r['label'],r['stage'],r['repeat']) for r in report['runs']}
    report['complete'] = all(key in found for key in required)
    save()
    assert report['complete'], 'Matrix is still incomplete; resume without stage filters.'
    print(f'MATRIX PASS {len(cases)} cases, {len(report["runs"])} stages; {report_file}', flush=True)


if __name__=='__main__': main()
