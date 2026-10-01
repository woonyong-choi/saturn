"""raw/의 묶음 기록을 복제별 지표와 묶음 표로 정리해 processed/에 쓴다."""

from __future__ import annotations

import argparse
import csv
import json
import logging
import sys
from collections import defaultdict
from pathlib import Path

logger = logging.getLogger(__name__)

LATE_FIRST_BLOCK = 51

POPULATION_FIELDS = (
    "condition",
    "signal_model",
    "wrong_rate_before",
    "wrong_rate_after",
    "delta_before",
    "delta_after",
    "oracle_threshold_after",
    "balance_threshold_after",
    "wrong_rate_at_low_after",
    "wrong_rate_at_high_after",
)
BLOCK_FIELDS = (
    "condition",
    "replicate",
    "block",
    "acted",
    "acted_wrong",
    "skipped_correct",
    "wrong_signals",
    "missed_signals",
    "asks",
    "restarts",
    "frozen_end",
    "threshold_mean",
    "threshold_min",
    "threshold_max",
    "threshold_end",
)
REPLICATE_FIELDS = (
    "condition",
    "replicate",
    "late_acted",
    "late_acted_wrong",
    "late_wrong_rate",
    "late_threshold_mean",
    "late_window_range",
    "late_wrong_signals",
    "late_missed_signals",
    "late_asks",
    "restarts",
    "late_restarts",
    "late_frozen_blocks",
    "frozen_final",
)


def main() -> int:
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
    parser = argparse.ArgumentParser()
    parser.add_argument("--raw-dir", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    population = [
        row for path in sorted(args.raw_dir.glob("population-*.jsonl")) for row in _read(path)
    ]
    blocks = [row for path in sorted(args.raw_dir.glob("sim-*.jsonl")) for row in _read(path)]
    if not population or not blocks:
        raise FileNotFoundError(f"no raw files in {args.raw_dir}")

    args.out_dir.mkdir(parents=True, exist_ok=True)
    _write(args.out_dir / "population.csv", POPULATION_FIELDS, population)
    blocks.sort(key=lambda row: (row["condition"], row["replicate"], row["block"]))
    _write(args.out_dir / "blocks.csv", BLOCK_FIELDS, blocks)

    grouped: dict[tuple[str, int], list[dict]] = defaultdict(list)
    for row in blocks:
        grouped[(row["condition"], row["replicate"])].append(row)
    replicates = [_summarize(condition, replicate, rows) for (condition, replicate), rows in grouped.items()]
    _write(args.out_dir / "replicates.csv", REPLICATE_FIELDS, replicates)
    logger.info("processed %d blocks, %d replicates", len(blocks), len(replicates))
    return 0


def _summarize(condition: str, replicate: int, rows: list[dict]) -> dict:
    late = [row for row in rows if row["block"] >= LATE_FIRST_BLOCK]
    acted = sum(row["acted"] for row in late)
    acted_wrong = sum(row["acted_wrong"] for row in late)
    return {
        "condition": condition,
        "replicate": replicate,
        "late_acted": acted,
        "late_acted_wrong": acted_wrong,
        "late_wrong_rate": round(acted_wrong / acted, 6) if acted else "",
        "late_threshold_mean": round(sum(row["threshold_mean"] for row in late) / len(late), 6),
        "late_window_range": round(
            sum(row["threshold_max"] - row["threshold_min"] for row in late) / len(late), 6
        ),
        "late_wrong_signals": sum(row["wrong_signals"] for row in late),
        "late_missed_signals": sum(row["missed_signals"] for row in late),
        "late_asks": sum(row["asks"] for row in late),
        "restarts": sum(row["restarts"] for row in rows),
        "late_restarts": sum(row["restarts"] for row in late),
        "late_frozen_blocks": sum(int(row["frozen_end"]) for row in late),
        "frozen_final": int(rows[-1]["frozen_end"]),
    }


def _read(path: Path) -> list[dict]:
    with path.open(encoding="utf-8") as file:
        return [json.loads(line) for line in file if line.strip()]


def _write(path: Path, fields: tuple[str, ...], rows: list[dict]) -> None:
    with path.open("w", encoding="utf-8", newline="") as file:
        writer = csv.DictWriter(file, fieldnames=fields, extrasaction="ignore", lineterminator="\n")
        writer.writeheader()
        for row in rows:
            writer.writerow({key: _cell(row[key]) for key in fields})


def _cell(value: object) -> object:
    if isinstance(value, bool):
        return int(value)
    return value


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        logger.exception("process failed")
        sys.exit(1)
