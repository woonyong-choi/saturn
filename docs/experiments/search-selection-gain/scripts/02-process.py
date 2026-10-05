"""원응답에서 trial을 다시 만들고 원자료 해시 목록을 남긴다."""

from __future__ import annotations

import json

import fixtures
import trials
from lock import LOCK
from runtime import CALLER, PRIVATE, PUBLIC, read

import importlib.util
_s = importlib.util.spec_from_file_location("collect", PUBLIC / "scripts/01-collect.py")


def providers(cases: list[dict]) -> dict[str, set]:
    collect = importlib.util.module_from_spec(_s)
    _s.loader.exec_module(collect)
    sealed = [c for c in cases if c["split"] == "sealed"]
    return {"claude": {c["task_id"] for c in sealed}, "codex": collect.codex_subset(sealed)}


def main() -> None:
    cases = read(PRIVATE / "cases.json")
    sealed = [c for c in cases if c["split"] == "sealed"]
    raw = trials.load_raw(PRIVATE / "raw")
    rows = trials.build_trials(sealed, providers(cases), raw, read(LOCK)["tau"])
    (PRIVATE / "trials.jsonl").write_text("".join(json.dumps(r, ensure_ascii=False, sort_keys=True) + "\n" for r in rows))
    sums = "".join(f"{trials.sha(raw[n])}  {n}\n" for n in sorted(raw))
    (PRIVATE / "SHA256SUMS").write_text(sums)
    if CALLER == "live":
        (PUBLIC / "data").mkdir(exist_ok=True)
        (PUBLIC / "data/SHA256SUMS").write_text(sums)
    print(f"processed {len(rows)} trials from {len(raw)} raw responses")


if __name__ == "__main__":
    main()
