"""사후 정답을 먼저 봉인한 뒤 모델별 병렬 통로에서 같은 입력을 평가한다."""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
import sys
import threading
from collections import Counter

from protocol import gold_prompt, parse, query_prompt
from runtime import MODELS, PRIVATE, call_cli, call_jev, is_rejected, read, setup, write

GOLD_STOP = threading.Event()


def check_samples() -> list[dict]:
    for name, checksum in read(PRIVATE / "sample-seal.json").items():
        if hashlib.sha256((PRIVATE / name).read_bytes()).hexdigest() != checksum:
            raise RuntimeError("sample seal mismatch: " + name)
    return read(PRIVATE / "samples.json")


def gold_call(item: tuple[dict, int]) -> dict:
    if GOLD_STOP.is_set():
        raise RuntimeError("gold model rejected")
    case, repeat = item
    trial = f"gold-sol-{case['sample_id']}-r{repeat}"
    record = call_cli("codex", MODELS["sol"], gold_prompt(case), trial)
    if is_rejected(record):
        GOLD_STOP.set()
        raise RuntimeError("gold authentication or model rejected")
    return dict(sample_id=case["sample_id"], repeat=repeat, **parse(record, "gold"))


def collect_gold(cases: list[dict]) -> None:
    target = PRIVATE / "gold.json"
    if target.exists() and (PRIVATE / "gold-seal.json").exists():
        print(json.dumps(dict(phase="gold", status="already_frozen")), flush=True)
        return
    jobs = [(case, repeat) for repeat in (1, 2) for case in cases]
    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        first = list(pool.map(gold_call, jobs))
    grouped = {case["sample_id"]: [] for case in cases}
    for row in first:
        grouped[row["sample_id"]].append(row)
    disagreements = [
        case
        for case in cases
        if len({r["value"]["label"] for r in grouped[case["sample_id"]] if r["valid"]})
        != 1
        or not all(r["valid"] for r in grouped[case["sample_id"]])
    ]
    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        for row in pool.map(gold_call, [(case, 3) for case in disagreements]):
            grouped[row["sample_id"]].append(row)
    final = []
    for sample_id, judgments in grouped.items():
        counts = Counter(row["value"]["label"] for row in judgments if row["valid"])
        label, votes = counts.most_common(1)[0] if counts else ("uncertain", 0)
        final.append(
            dict(
                sample_id=sample_id,
                label=label if votes >= 2 else "uncertain",
                judgments=judgments,
            )
        )
    write(target, final)
    write(
        PRIVATE / "gold-seal.json",
        {"gold.json": hashlib.sha256(target.read_bytes()).hexdigest()},
    )
    print(
        json.dumps(
            dict(
                phase="gold",
                completed=len(final),
                third_votes=len(disagreements),
                labels=dict(Counter(row["label"] for row in final)),
            )
        ),
        flush=True,
    )


def collect_lane(name: str, cases: list[dict]) -> None:
    model = MODELS.get(name)
    if name in ("sonnet", "opus", "haiku"):
        discovery = next(
            row for row in read(PRIVATE / "claude-models.json") if row["alias"] == name
        )
        if len(discovery["models"]) != 1:
            print(
                json.dumps(dict(model=name, excluded="unconfirmed model")), flush=True
            )
            return
        model = discovery["models"][0]
    for index, case in enumerate(cases):
        if name == "jev":
            records = []
            for repeat in (1, 2, 3):
                record = call_jev(
                    case["state"], f"query-jev-{case['sample_id']}-r{repeat}"
                )
                records.append(record)
                if is_rejected(record):
                    raise RuntimeError("judge authentication or model rejected")
        else:
            kind = "codex" if name in MODELS else "claude"
            records = [
                call_cli(
                    kind,
                    model,
                    query_prompt(case),
                    f"query-{name}-{case['sample_id']}-r1",
                )
            ]
        if any(is_rejected(row) for row in records):
            raise RuntimeError("cli authentication or model rejected: " + name)
        if (index + 1) % 10 == 0:
            print(
                json.dumps(dict(phase="query", model=name, completed=index + 1)),
                flush=True,
            )


def main() -> None:
    setup()
    cases = check_samples()
    phase = sys.argv[1] if len(sys.argv) > 1 else "all"
    if phase in ("gold", "all"):
        collect_gold(cases)
    if phase in ("query", "all"):
        for name, checksum in read(PRIVATE / "gold-seal.json").items():
            if hashlib.sha256((PRIVATE / name).read_bytes()).hexdigest() != checksum:
                raise RuntimeError("gold seal mismatch")
        names = ["sonnet", "opus", "haiku", *MODELS, "jev"]
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(names)) as pool:
            futures = [pool.submit(collect_lane, name, cases) for name in names]
            for future in concurrent.futures.as_completed(futures):
                future.result()


if __name__ == "__main__":
    main()
