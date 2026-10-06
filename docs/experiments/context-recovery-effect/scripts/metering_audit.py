"""기존 원자료의 기본 압축 호출 사용량을 별도로 대조한다."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json

import analyze
import collect


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    rows = []
    for path in sorted(
        (collect.BASE.parent / "data/raw").glob("formal-*-provider.json.gz")
    ):
        raw = json.load(gzip.open(path, "rt"))
        store = raw["store"]
        inputs = {i["id"] for i in store["inputs"] if i["text"] == "/compact"}
        runs = {r["id"] for r in store["runs_meta"] if r["input_id"] in inputs}
        usage = analyze.PROCESS.window_usage(store, runs)
        rows.append(
            {
                "trial": raw["trial"],
                "track": raw["provider"],
                "source_file": path.name,
                "source_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "compact_runs": len(runs),
                "reported_compaction_tokens": sum(
                    sum(v.values()) for v in usage.values()
                ),
            }
        )
    if len(rows) != 44:
        raise ValueError("expected prior native compaction records unavailable")
    dest = collect.EXP / "data/prior-compaction-metering.json"
    text = json.dumps(rows, sort_keys=True, indent=2) + "\n"
    if args.verify:
        if dest.read_text() != text:
            raise ValueError("prior metering audit differs")
    else:
        dest.write_text(text)
    print(
        "prior metering records",
        len(rows),
        "zero delta",
        sum(r["reported_compaction_tokens"] == 0 for r in rows),
    )


if __name__ == "__main__":
    main()
