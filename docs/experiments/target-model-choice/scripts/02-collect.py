"""정답을 먼저 봉인한 뒤 모델별 독립 질의를 재개 가능하게 실행한다."""

import json
import sys
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
from protocol import (
    REFERENCE,
    SOL,
    choice_prompt,
    gold_key,
    gold_prompt,
    make_body,
    normalize_gold,
)
from runtime import (
    PRIVATE,
    PUBLIC,
    call_cli,
    call_jev,
    digest,
    parse_json,
    read,
    response_text,
    setup,
    write,
)


def validate_seal(name):
    for path, checksum in read(PRIVATE / name).items():
        if digest(PRIVATE / path) != checksum:
            raise RuntimeError("sealed input changed: " + path)


def label_case(case):
    trial = "gold-" + case["sample_id"]
    labels = []
    for repeat in (1, 2, 3):
        if (
            repeat == 3
            and labels[0] is not None
            and gold_key(labels[0]) == gold_key(labels[1])
        ):
            break
        record = call_cli("codex", SOL, gold_prompt(case), trial + "-" + str(repeat))
        text, meta = response_text(record)
        value = None if meta.get("tool_events") else normalize_gold(parse_json(text))
        labels.append(value)
    counts = Counter(gold_key(v) for v in labels if v is not None)
    winner = next((key for key, n in counts.items() if n >= 2), None)
    chosen = next((v for v in labels if gold_key(v) == winner), None)
    return dict(
        sample_id=case["sample_id"],
        first_agree=labels[0] is not None
        and gold_key(labels[0]) == gold_key(labels[1]),
        ballots=labels,
        final=chosen,
    )


def collect_gold(samples):
    target = PRIVATE / "labels.json"
    if target.exists():
        validate_seal("gold-seal.json")
        return
    results = []
    with ThreadPoolExecutor(max_workers=6) as pool:
        futures = [pool.submit(label_case, case) for case in samples]
        for future in as_completed(futures):
            results.append(future.result())
            print(
                json.dumps(
                    dict(stage="gold", completed=len(results), total=len(samples))
                ),
                flush=True,
            )
    results.sort(key=lambda r: r["sample_id"])
    write(target, results)
    write(
        PRIVATE / "gold-seal.json",
        {
            "labels.json": digest(target),
            "samples.json": digest(PRIVATE / "samples.json"),
        },
    )


def collect_lane(lane, samples):
    for number, case in enumerate(samples, 1):
        for repeat in range(1, 4 if lane == "jev" else 2):
            trial = lane + "-" + case["sample_id"] + "-" + str(repeat)
            if lane == "jev":
                record = call_jev(make_body(case), trial)
            else:
                kind, model = REFERENCE[lane]
                record = call_cli(kind, model, choice_prompt(case), trial)
            if record.get("http_status") in (401, 403):
                raise RuntimeError("router authentication rejected")
            if record.get("status") == "process_error":
                raise RuntimeError("CLI lane stopped after process error: " + lane)
        print(
            json.dumps(dict(stage=lane, completed=number, total=len(samples))),
            flush=True,
        )


def main():
    setup()
    validate_seal("sample-seal.json")
    seal = read(PRIVATE / "design-seal.json")
    for name, checksum in seal["files"].items():
        if digest(PUBLIC / name) != checksum:
            raise RuntimeError("preregistered file changed: " + name)
    samples = read(PRIVATE / "samples.json")
    phase = sys.argv[1]
    if phase in ("gold", "all"):
        collect_gold(samples)
    if phase in ("query", "all"):
        validate_seal("gold-seal.json")
        with ThreadPoolExecutor(max_workers=5) as pool:
            futures = [
                pool.submit(collect_lane, lane, samples) for lane in ["jev", *REFERENCE]
            ]
            for future in as_completed(futures):
                future.result()


if __name__ == "__main__":
    main()
