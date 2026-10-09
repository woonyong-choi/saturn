"""Astra 용량·형식 실패 묶음만 원응답을 보존한 채 재시도한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"script unavailable: {path.name}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ret = load("retrospective", HERE / "retrospective.py")
audit = load("retrospective_analyze", HERE / "retrospective-analyze.py")
QUALITY = ret.OUT / "quality1000"


def prepare(group: str, attempt: int) -> None:
    source = QUALITY / group
    cases = json.loads((source / "cases.json").read_text())
    _, labels, source_audit = audit.load_labels(source)
    for sid in source_audit["bad_evidence"]:
        labels.pop(sid, None)
    for number in range(1, attempt):
        previous = source / f"retry{number}"
        if previous.exists():
            _, recovered, retry_audit = audit.load_labels(previous)
            for sid in retry_audit["bad_evidence"]:
                recovered.pop(sid, None)
            labels.update(recovered)
    pending = [case for case in cases if case["id"] not in labels]
    destination = source / f"retry{attempt}"
    ret.write_once(destination / "cases.json", pending)
    ret.write_once(
        destination / "retry-plan.json",
        {
            "ts_utc": ret.now(),
            "source_cases": len(cases),
            "already_labeled": len(cases) - len(pending),
            "retry_cases": len(pending),
            "source": group,
            "attempt": attempt,
        },
    )
    print(json.dumps({"group": group, "attempt": attempt, "retry_cases": len(pending)}))


def label(group: str, attempt: int) -> None:
    ret.OUT = QUALITY / group / f"retry{attempt}"
    if not (ret.OUT / "seal.json").exists():
        ret.seal()
    jobs = ret.batches(json.loads((ret.OUT / "cases.json").read_text()))
    for _ in jobs:
        ret.label(1)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("prepare", "label"))
    parser.add_argument(
        "--group", required=True, choices=("primary", "secondary", "third")
    )
    parser.add_argument("--attempt", type=int, default=1)
    args = parser.parse_args()
    if args.attempt not in (1, 2, 3):
        parser.error("attempt must be 1, 2, or 3")
    if args.action == "prepare":
        prepare(args.group, args.attempt)
    else:
        label(args.group, args.attempt)


if __name__ == "__main__":
    main()
