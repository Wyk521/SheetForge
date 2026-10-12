#!/usr/bin/env python3
"""Run fixed CSV/XLSX fixtures three times; compare elapsed time, RSS and cell XML."""
import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import sys
import zipfile


def metrics(log):
    if sys.platform == "darwin":
        seconds = float(re.search(r"([\d.]+) real", log)[1])
        rss = int(re.search(r"(\d+)\s+maximum resident set size", log)[1])
    else:
        elapsed = re.search(r"Elapsed \(wall clock\) time .*: ([\d:.]+)", log)[1]
        seconds = 0.0
        for part in elapsed.split(":"):
            seconds = seconds * 60 + float(part)
        rss = int(re.search(r"Maximum resident set size \(kbytes\): (\d+)", log)[1]) * 1024
    return {"seconds": seconds, "rss_bytes": rss}


def cells(path):
    result = {}
    with zipfile.ZipFile(path) as archive:
        for name in sorted(archive.namelist()):
            if name.startswith("xl/worksheets/") or name in ("xl/sharedStrings.xml", "xl/styles.xml", "xl/workbook.xml"):
                digest = hashlib.sha256()
                with archive.open(name) as stream:
                    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                        digest.update(chunk)
                result[name] = digest.hexdigest()
    return result


def file_hash(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("label")
    parser.add_argument("--compare")
    parser.add_argument("--generate", action="store_true")
    parser.add_argument("--repo", type=Path, default=Path.cwd(),
                        help="checkout to build; lets this script benchmark revisions that predate it")
    args = parser.parse_args()
    args.directory = args.directory.resolve()
    args.directory.mkdir(parents=True, exist_ok=True)
    args.repo = args.repo.resolve()
    build = subprocess.check_output(["cargo", "test", "--manifest-path", "src-tauri/Cargo.toml", "--locked", "--no-run", "--message-format=json"], cwd=args.repo, text=True)
    binary = next(item["executable"] for line in build.splitlines() if (item := json.loads(line)).get("executable"))
    env = dict(os.environ, SHEETFORGE_BENCH_DIR=str(args.directory))
    if args.generate:
        subprocess.run([binary, "--exact", "performance_tests::generate_fixtures", "--ignored"], env=env, check=True)
    command = lambda *parts: subprocess.check_output(parts, cwd=args.repo, text=True).strip()
    report = {"metadata": {
        "source_commit": command("git", "rev-parse", "HEAD"),
        "source_dirty": bool(command("git", "status", "--porcelain", "--untracked-files=no")),
        "rustc": command("rustc", "--version"),
        "cargo": command("cargo", "--version"),
        "platform": platform.platform(),
        "build_profile": "test-debug",
        "fixtures": {name: file_hash(args.directory / name) for name in ("million.csv", "multi.xlsx")},
    }}
    for kind in ("csv", "xlsx"):
        runs = []
        for index in range(3):
            log = args.directory / f"{args.label}-{kind}-{index}.txt"
            with log.open("w") as output:
                subprocess.run(["/usr/bin/time", "-l" if sys.platform == "darwin" else "-v", binary, "--exact", "performance_tests::merge_fixture", "--ignored"], env=dict(env, SHEETFORGE_BENCH_KIND=kind), stdout=output, stderr=output, check=True)
            result = metrics(log.read_text())
            runs.append(result)
            print(kind, index + 1, result, flush=True)
        snapshot = args.directory / f"{args.label}-{kind}-output.xlsx"
        shutil.copyfile(args.directory / f"{kind}-output.xlsx", snapshot)
        report[kind] = {"runs": runs, "median_seconds": statistics.median(r["seconds"] for r in runs), "median_rss_bytes": statistics.median(r["rss_bytes"] for r in runs), "cells": cells(snapshot)}
    (args.directory / f"{args.label}.json").write_text(json.dumps(report, indent=2))
    if args.compare:
        previous = json.loads((args.directory / f"{args.compare}.json").read_text())
        if "metadata" in previous:
            for key in ("rustc", "cargo", "platform", "build_profile", "fixtures"):
                assert report["metadata"][key] == previous["metadata"][key], f"benchmark conditions changed: {key}"
        for kind in ("csv", "xlsx"):
            current = report[kind]
            assert current["cells"] == previous[kind]["cells"], f"{kind}: output cells changed"
            for metric in ("median_seconds", "median_rss_bytes"):
                ratio = current[metric] / previous[kind][metric]
                print(f"{kind} {metric}: {ratio - 1:+.1%}")
                assert ratio <= 1.10, f"{kind}: {metric} regressed by more than 10%"


if __name__ == "__main__":
    main()
