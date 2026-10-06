#!/usr/bin/env python3
"""원자료에서 표를 다시 만들어 대조하고, 분석을 두 번 실행해 바이트가 같은지, 키 문자열이 없는지 확인한다."""
from __future__ import annotations

import gzip
import importlib.util
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import tasks  # noqa: E402

BASE = HERE.parent
DATA = BASE / "data"


def load(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def fail(message: str) -> None:
    print("FAIL:", message)
    raise SystemExit(1)


def main() -> None:
    process = load("process", "02-process.py")
    analyze = load("analyze", "03-analyze.py")
    key = subprocess.run(["security", "find-generic-password", "-s", "saturn-verify-router", "-w"], capture_output=True, text=True, check=True).stdout.strip()
    leaks = 0
    for phase in ("dev", "confirm"):
        raw_dir = DATA / "raw" / phase
        if not raw_dir.exists():
            continue
        by_id = {t["task_id"]: t for t in tasks.make_tasks(phase)}
        files = sorted(raw_dir.glob("*.json.gz"))
        rows = []
        seen = set()
        for f in files:
            blob = gzip.decompress(f.read_bytes()).decode("utf-8")
            leaks += blob.count(key)
            data = json.loads(blob)
            if (data["task_id"], data["arm"]) in seen:
                fail("duplicate trial " + f.name)
            seen.add((data["task_id"], data["arm"]))
            rows.append(process.build_row(phase, data, by_id))
            if data["status"] == "ok" and not data["tables"].get("usage"):
                fail("ok trial without usage: " + f.name)
            ids = [u["id"] for u in data["tables"].get("usage", [])]
            if len(ids) != len(set(ids)):
                fail("duplicate usage id: " + f.name)
        expected = {(t, a) for t in by_id for a in ("c-haiku", "c-sonnet", "c-opus", "c-auto", "x-luna", "x-astra", "x-auto", "b-auto")}
        if seen != expected:
            fail(f"{phase}: trial set differs from the sample ({len(seen)} of {len(expected)})")
        rows.sort(key=lambda r: r["tid"])
        written = (DATA / "processed" / f"trials-{phase}.json").read_text(encoding="utf-8")
        if written != json.dumps(rows, ensure_ascii=False, sort_keys=True, indent=1) + "\n":
            fail(f"{phase}: processed trials differ from the raw data")
        for r in rows:
            if r["mode"] == "auto" and r["selection"] and (r["selection"]["source"] == "router") != r["target_applied"]:
                fail(f"selection source disagrees with the router confidence: {r['task_id']} {r['arm']}")
        first = json.dumps(analyze.analyze(phase, rows, json.loads((BASE / "frozen.json").read_text())["delta_cost"] if (BASE / "frozen.json").exists() else analyze.DELTA_COST), ensure_ascii=False, sort_keys=True, indent=1) + "\n"
        second = json.dumps(analyze.analyze(phase, json.loads(written), json.loads((BASE / "frozen.json").read_text())["delta_cost"] if (BASE / "frozen.json").exists() else analyze.DELTA_COST), ensure_ascii=False, sort_keys=True, indent=1) + "\n"
        if first != second or first != (BASE / "results" / f"summary-{phase}.json").read_text(encoding="utf-8"):
            fail(f"{phase}: analysis output differs between runs")
        print(f"{phase}: {len(rows)} trials rebuilt, analysis identical")
    for path in list((BASE / "results").glob("*")) + list((DATA / "processed").glob("*")):
        leaks += path.read_text(encoding="utf-8").count(key)
    print("router key occurrences in raw data, processed tables and results:", leaks)
    if leaks:
        fail("router key string found")
    problems = tasks.selftest(HERE.parents[3] / ".runtime" / "st")
    if problems:
        fail("task selftest: " + ", ".join(problems))
    print("task selftest passed")


if __name__ == "__main__":
    main()
