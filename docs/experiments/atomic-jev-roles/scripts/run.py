"""Narrow Jev role development experiment; raw user inputs stay in .local."""

from __future__ import annotations

import concurrent.futures
import getpass
import hashlib
import json
import math
import os
import random
import re
import sys
import time
import urllib.error
import urllib.request
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parents[1]
PRIVATE = ROOT / ".local/experiments/atomic-jev-roles"
SOURCE = Path(
    os.environ.get(
        "SATURN_HUMAN_REVIEW_DIR", ROOT / ".local/experiments/constraint-deep"
    )
)
MODEL = "jev-1.13.0"
URL = "https://api.typesafe.ai/v1/systemone"
QUESTIONS = {
    "constraint": "Does the user's latest input state a rule intended to persist across separate future tasks, rather than only the current task?",
    "same_observation": "Do these two observations describe the same test in the same run with the same result?",
}
TOPICS = [
    ("cache.rs", "재시도", "빌드"),
    ("parser.rs", "공백", "검사"),
    ("queue.rs", "순서", "빌드"),
    ("store.rs", "잠금", "검사"),
    ("router.rs", "선택", "빌드"),
    ("engine.rs", "종료", "검사"),
    ("event.rs", "중복", "빌드"),
    ("packet.rs", "예산", "검사"),
    ("socket.rs", "연결", "빌드"),
    ("usage.rs", "집계", "검사"),
    ("model.rs", "목록", "빌드"),
    ("input.rs", "전송", "검사"),
    ("session.rs", "복구", "빌드"),
    ("task.rs", "대기", "검사"),
    ("prompt.rs", "형식", "빌드"),
    ("trace.rs", "기록", "검사"),
    ("policy.rs", "권한", "빌드"),
    ("switch.rs", "전환", "검사"),
    ("filter.rs", "제외", "빌드"),
    ("history.rs", "조회", "검사"),
    ("result.rs", "반환", "빌드"),
    ("token.rs", "계산", "검사"),
    ("config.rs", "적용", "빌드"),
    ("clock.rs", "만료", "검사"),
    ("agent.rs", "생성", "빌드"),
    ("home.rs", "경로", "검사"),
    ("state.rs", "변경", "빌드"),
    ("write.rs", "저장", "검사"),
    ("read.rs", "해석", "빌드"),
    ("status.rs", "표시", "검사"),
]


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def load_json(path: Path) -> object:
    return json.loads(path.read_text())


def generate_observations() -> list[dict]:
    items = []
    for topic, (path, test, kind) in enumerate(TOPICS):
        run = 200 + topic
        first = f"{kind} {run}에서 {path}의 {test} 테스트가 시간 제한으로 실패했다."
        pairs = [
            (
                f"{run}번 {kind}의 {path} {test} 검사는 제한 시간을 넘어 통과하지 못했다.",
                True,
                "paraphrase",
            ),
            (
                f"{run}번 {kind}의 {path} {test} 검사는 다시 실행해 통과했다.",
                False,
                "opposite_result",
            ),
            (
                f"{run + 1}번 {kind}의 {path} {test} 검사는 제한 시간을 넘어 통과하지 못했다.",
                False,
                "different_run",
            ),
            (
                f"{run}번 {kind}의 {path} 다른 검사는 제한 시간을 넘어 통과하지 못했다.",
                False,
                "different_test",
            ),
        ]
        for variant, (second, label, kind_name) in enumerate(pairs):
            items.append(
                {
                    "id": f"o{topic:02d}-{variant}",
                    "role": "same_observation",
                    "topic": topic,
                    "split": "development" if topic < 10 else "confirm",
                    "label": label,
                    "kind": kind_name,
                    "state": f"Observation A: {first}\nObservation B: {second}",
                }
            )
    return items


def prepare() -> None:
    PRIVATE.mkdir(parents=True, exist_ok=True)
    sample = PRIVATE / "sample.json"
    if sample.exists():
        return
    human_path = SOURCE / "human-review-result.json"
    review_path = SOURCE / "human-review.jsonl"
    human = {row["case_id"]: row["verdict"] for row in load_json(human_path)["results"]}
    items = []
    for line in review_path.open():
        row = json.loads(line)
        case_id = row["conversation_id"] + ":" + row["turn"]["turn_id"]
        if case_id not in human:
            continue
        items.append(
            {
                "id": "c-" + hashlib.sha256(case_id.encode()).hexdigest()[:12],
                "role": "constraint",
                "label": human[case_id],
                "old_score": row["probability"],
                "state": "Latest user input: " + row["turn"]["text"],
            }
        )
    if len(items) != 40:
        raise RuntimeError("human sample count changed")
    items += generate_observations()
    random.Random(54607).shuffle(items)
    write_json(sample, items)
    write_json(
        PRIVATE / "source.json",
        {
            "review_sha256": sha(review_path),
            "human_sha256": sha(human_path),
            "sample_sha256": sha(sample),
            "design_commit": os.popen("git rev-parse --short HEAD").read().strip(),
        },
    )


def rows(path: Path) -> list[dict]:
    if not path.exists():
        return []
    return [json.loads(line) for line in path.open() if line.strip()]


def append(path: Path, row: dict) -> None:
    with path.open("a") as file:
        file.write(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n")
        file.flush()
        os.fsync(file.fileno())


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise urllib.error.HTTPError(req.full_url, code, "redirect denied", headers, fp)


def ask(item: dict, repeat: int, key: str) -> dict:
    request = {
        "model": MODEL,
        "state": item["state"],
        "questions": {
            "judgment": {"type": "noul", "instructions": QUESTIONS[item["role"]]}
        },
    }
    encoded = json.dumps(request, ensure_ascii=False, separators=(",", ":")).encode()
    result = {
        "id": item["id"],
        "role": item["role"],
        "repeat": repeat,
        "request": request,
        "request_bytes": len(encoded),
        "at_utc": datetime.now(timezone.utc).isoformat(),
    }
    if len(encoded) > 110000:
        return {**result, "status": "oversize"}
    req = urllib.request.Request(
        URL,
        encoded,
        method="POST",
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"},
    )
    start = time.monotonic()
    try:
        with urllib.request.build_opener(NoRedirect).open(req, timeout=45) as response:
            body = response.read().decode(errors="replace").replace(key, "[secret]")
            result.update(http_status=response.status, raw_response=body)
            parsed = json.loads(body)
            value = parsed["answers"]["judgment"]["noul"]
            if (
                type(value) not in (int, float)
                or not math.isfinite(value)
                or not 0 <= value <= 1
            ):
                raise ValueError("invalid probability")
            result.update(
                status="ok",
                score=value,
                model=parsed.get("model"),
                usage=parsed.get("usage"),
            )
    except urllib.error.HTTPError as error:
        result.update(
            status="http_error",
            http_status=error.code,
            raw_response=error.read().decode(errors="replace").replace(key, "[secret]"),
        )
    except (OSError, ValueError, KeyError, TypeError) as error:
        result.update(status="failed", error=type(error).__name__)
    result["latency_ms"] = round((time.monotonic() - start) * 1000, 3)
    return result


def collect() -> None:
    sample = load_json(PRIVATE / "sample.json")
    done = {(x["id"], x["repeat"]) for x in rows(PRIVATE / "responses.jsonl")}
    reserved = {(x["id"], x["repeat"]) for x in rows(PRIVATE / "reservations.jsonl")}
    if done or reserved:
        raise RuntimeError("collection already started; do not resend reserved trials")
    key = getpass.getpass("Jev key (not stored): ")
    if not key:
        raise RuntimeError("missing key")
    tasks = [(item, repeat) for item in sample for repeat in (1, 2)]
    random.Random(54607).shuffle(tasks)
    for batch in [tasks[:1], tasks[1:]]:
        for item, repeat in batch:
            append(PRIVATE / "reservations.jsonl", {"id": item["id"], "repeat": repeat})
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            futures = [pool.submit(ask, item, repeat, key) for item, repeat in batch]
            seen_auth_error = False
            for future in concurrent.futures.as_completed(futures):
                result = future.result()
                append(PRIVATE / "responses.jsonl", result)
                seen_auth_error |= result.get("http_status") in (401, 403)
        if seen_auth_error:
            raise RuntimeError("Jev authentication rejected")
    print("saved", len(tasks), "responses")


def grams(value: str) -> set[str]:
    value = re.sub(r"\s+", "", value.lower())
    return {value[i : i + 3] for i in range(max(0, len(value) - 2))}


def similarity(state: str) -> float:
    left, right = state.split("\n", 1)
    a, b = grams(left), grams(right)
    return len(a & b) / len(a | b) if a | b else 1.0


def structured_signature(text: str) -> tuple[str | None, str | None, str | None, str]:
    """Post-hoc parser for the generated format, not a preregistered baseline."""
    run = re.search(r"\d+", text)
    path = re.search(r"[a-z]+\.rs", text)
    test = next((name for _, name, _ in TOPICS if name in text), None)
    result = (
        "failed"
        if "실패" in text or "통과하지 못" in text or "넘어" in text
        else "passed"
        if "통과했다" in text
        else "unknown"
    )
    return (run.group() if run else None, path.group() if path else None, test, result)


def wilson(k: int, n: int) -> list[float]:
    if n == 0:
        return [0, 1]
    z = 1.959963984540054
    p = k / n
    d = 1 + z * z / n
    center = (p + z * z / (2 * n)) / d
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return [center - half, center + half]


def bin_score(row: dict | None) -> str:
    if not row or row.get("status") != "ok":
        return "failed"
    score = row["score"]
    return "low" if score <= 0.2 else "high" if score >= 0.8 else "middle"


def analyze() -> None:
    sample = load_json(PRIVATE / "sample.json")
    responses = {(r["id"], r["repeat"]): r for r in rows(PRIVATE / "responses.jsonl")}
    if len(responses) != 2 * len(sample):
        raise RuntimeError("response coverage incomplete")
    dev = [
        x
        for x in sample
        if x["role"] == "same_observation" and x["split"] == "development"
    ]
    candidates = [i / 100 for i in range(20, 81, 5)]
    threshold = min(
        candidates,
        key=lambda t: (
            -sum((similarity(x["state"]) >= t) == x["label"] for x in dev),
            t,
        ),
    )
    real = [x for x in sample if x["role"] == "constraint" and x["label"] != "unsure"]
    old_correct = sum(
        (x["old_score"] >= 0.5) == (x["label"] == "constraint") for x in real
    )
    new_correct = sum(
        (responses[(x["id"], 1)].get("score", -1) >= 0.5)
        == (x["label"] == "constraint")
        and responses[(x["id"], 1)]["status"] == "ok"
        for x in real
    )
    human_pairs = Counter()
    for item in real:
        label = item["label"] == "constraint"
        old_hit = (item["old_score"] >= 0.5) == label
        new_row = responses[(item["id"], 1)]
        new_hit = new_row["status"] == "ok" and (new_row["score"] >= 0.5) == label
        human_pairs[(old_hit, new_hit)] += 1
    unsure = [x for x in sample if x["role"] == "constraint" and x["label"] == "unsure"]
    unsure_positive = sum(
        responses[(x["id"], 1)].get("score", -1) >= 0.5 for x in unsure
    )
    confirm = [
        x for x in sample if x["role"] == "same_observation" and x["split"] == "confirm"
    ]
    grouped = defaultdict(list)
    wins = Counter()
    misses = Counter()
    by_kind = defaultdict(Counter)
    structured_correct = 0
    for item in confirm:
        response = responses[(item["id"], 1)]
        jev = response.get("score", -1) >= 0.5 if response["status"] == "ok" else None
        code = similarity(item["state"]) >= threshold
        jc = jev == item["label"]
        cc = code == item["label"]
        wins[(jc, cc)] += 1
        misses["jev"] += item["label"] and not jc
        misses["code"] += item["label"] and not cc
        by_kind[item["kind"]]["count"] += 1
        by_kind[item["kind"]]["jev_positive_05"] += jev is True
        by_kind[item["kind"]]["jev_positive_08"] += response.get("score", -1) >= 0.8
        grouped[item["topic"]].append((int(jc), int(cc)))
        left, right = item["state"].split("\n", 1)
        structured_correct += (
            structured_signature(left) == structured_signature(right)
        ) == item["label"]
    ids = sorted(grouped)
    rng = random.Random(54607)
    differences = []
    for _ in range(2000):
        draw = [rng.choice(ids) for _ in ids]
        pairs = [v for topic in draw for v in grouped[topic]]
        differences.append(sum(a - b for a, b in pairs) / len(pairs))
    differences.sort()
    agreement = sum(
        bin_score(responses[(x["id"], 1)]) == bin_score(responses[(x["id"], 2)])
        for x in sample
    )
    statuses = Counter(r["status"] for r in responses.values())
    tokens = Counter()
    for r in responses.values():
        usage = r.get("usage") or {}
        for name in ("input_tokens", "output_tokens"):
            if isinstance(usage.get(name), int):
                tokens[name] += usage[name]
    value = {
        "flow": {
            "items": len(sample),
            "calls": len(responses),
            "statuses": dict(statuses),
            "human_decisive": len(real),
            "human_unsure": len(sample) - len(confirm) - len(dev) - len(real),
            "observation_development": len(dev),
            "observation_confirm": len(confirm),
        },
        "human": {
            "old_correct": old_correct,
            "new_correct": new_correct,
            "n": len(real),
            "old_only": human_pairs[(True, False)],
            "new_only": human_pairs[(False, True)],
            "unsure_positive": unsure_positive,
        },
        "observation": {
            "code_threshold": threshold,
            "jev_correct": wins[(True, True)] + wins[(True, False)],
            "code_correct": wins[(True, True)] + wins[(False, True)],
            "n": len(confirm),
            "jev_only": wins[(True, False)],
            "code_only": wins[(False, True)],
            "jev_misses": misses["jev"],
            "code_misses": misses["code"],
            "difference_ci95": [differences[49], differences[1949]],
            "by_kind": {name: dict(counts) for name, counts in sorted(by_kind.items())},
            "exploratory_08_correct": sum(
                (responses[(x["id"], 1)].get("score", -1) >= 0.8) == x["label"]
                for x in confirm
            ),
            "exploratory_structured_correct": structured_correct,
        },
        "repeat": {
            "agree": agreement,
            "n": len(sample),
            "ci95": wilson(agreement, len(sample)),
        },
        "usage": dict(tokens),
        "latency_median_ms": sorted(r["latency_ms"] for r in responses.values())[
            len(responses) // 2
        ],
        "latency_p95_ms": sorted(r["latency_ms"] for r in responses.values())[
            math.ceil(len(responses) * 0.95) - 1
        ],
        "models": dict(Counter(r.get("model") for r in responses.values())),
    }
    write_json(HERE / "results/summary.json", value)
    print(json.dumps(value, ensure_ascii=False))


def verify() -> None:
    source = load_json(PRIVATE / "source.json")
    if source["sample_sha256"] != sha(PRIVATE / "sample.json"):
        raise RuntimeError("sample changed")
    for key, name in (
        ("review_sha256", "human-review.jsonl"),
        ("human_sha256", "human-review-result.json"),
    ):
        if source[key] != sha(SOURCE / name):
            raise RuntimeError("source changed: " + name)
    reservations = rows(PRIVATE / "reservations.jsonl")
    responses = rows(PRIVATE / "responses.jsonl")
    keys = [(x["id"], x["repeat"]) for x in reservations]
    response_keys = [(x["id"], x["repeat"]) for x in responses]
    if (
        len(keys) != len(set(keys))
        or len(response_keys) != len(set(response_keys))
        or set(keys) != set(response_keys)
    ):
        raise RuntimeError("reservation and response mismatch")
    if any(
        "apikey_" in (PRIVATE / name).read_text()
        for name in ("sample.json", "responses.jsonl")
    ):
        raise RuntimeError("secret-shaped string in raw data")
    before = (HERE / "results/summary.json").read_bytes()
    analyze()
    if before != (HERE / "results/summary.json").read_bytes():
        raise RuntimeError("summary not reproducible")
    print("verified", len(responses), "responses")


if __name__ == "__main__":
    {"prepare": prepare, "collect": collect, "analyze": analyze, "verify": verify}[
        sys.argv[1]
    ]()
