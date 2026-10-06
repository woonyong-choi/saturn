"""원응답에서 trial을 다시 만들고 원자료 해시 목록을 남긴다."""

from __future__ import annotations

import json

import trials
from arm_runtime import CALLER, PRIVATE, PUBLIC, read

LIVE = CALLER == "live"


def main() -> None:
    cases = read(PRIVATE / "cases.json")
    raw = trials.load_raw()
    rows = trials.build_trials(cases, raw)
    (PRIVATE / "trials.jsonl").write_text(
        "".join(json.dumps(r, ensure_ascii=False, sort_keys=True) + "\n" for r in rows)
    )
    sums = "".join(f"{trials.sha(raw[n])}  {n}\n" for n in sorted(raw))
    (PRIVATE / "SHA256SUMS").write_text(sums)
    if LIVE:
        (PUBLIC / "data").mkdir(exist_ok=True)
        (PUBLIC / "data/SHA256SUMS").write_text(sums)
    print(f"processed {len(rows)} trials from {len(raw)} raw responses")


if __name__ == "__main__":
    main()
