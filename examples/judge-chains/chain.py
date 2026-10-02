"""judge 사슬 데모의 공용 실행기.

기록 칸은 docs/design/judge-chains.md의 사슬 기록을 그대로 쓴다.
외부 패키지 없이 표준 라이브러리만 쓴다.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import sys
import time
import urllib.error
import urllib.request
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

API_URL = "https://api.typesafe.ai/v1/systemone"
KEY_ENV = "TYPESAFE_API_KEY"
DEFAULT_MODEL = "jev-1.13"
# 입력 토큰 단가. 출력 토큰은 무료다(docs.typesafe.ai/models).
USD_PER_INPUT_TOKEN = 0.042 / 1_000_000
HERE = Path(__file__).resolve().parent
RUNS_DIR = HERE / "runs"


# --- 질문 생성 -------------------------------------------------------------


def noul(instructions: Any, true: Any = None, false: Any = None) -> dict:
    question: dict[str, Any] = {"type": "noul", "instructions": instructions}
    if true is not None or false is not None:
        question["criteria"] = {"true": true, "false": false}
    return question


def choice(instructions: Any, options: dict[str, Any]) -> dict:
    if len(options) > 255:
        raise ValueError(f"choice options exceed 255: {len(options)}")
    return {"type": "choice", "instructions": instructions, "criteria": options}


def score(instructions: Any, levels: list[Any]) -> dict:
    if not 2 <= len(levels) <= 10:
        raise ValueError(f"score levels must be 2..10: {len(levels)}")
    return {"type": "score", "instructions": instructions, "criteria": levels}


# --- 판단 계산 -------------------------------------------------------------


def confidence(probabilities: dict[str, float]) -> float:
    """(N·pmax − 1)/(N − 1). 균등 분포면 0, 한 선택지가 1이면 1."""
    n = len(probabilities)
    if n < 2:
        return 1.0
    return max(0.0, (n * max(probabilities.values()) - 1) / (n - 1))


def rollup(probabilities: dict[str, float], parent_of: dict[str, str]) -> dict[str, float]:
    """잎 확률을 더해 상위 노드 확률을 만든다. 추가 호출이 없다."""
    parents: dict[str, float] = {}
    for leaf, p in probabilities.items():
        parent = parent_of.get(leaf, leaf)
        parents[parent] = parents.get(parent, 0.0) + p
    return parents


def argmax(probabilities: dict[str, float]) -> str:
    return max(probabilities, key=probabilities.get)


@dataclass
class Backoff:
    label: str
    level: int  # 0 = 잎, 1 = 한 층 위, -1 = 대체 규칙
    confidence: float


def backoff(
    probabilities: dict[str, float],
    parents: list[dict[str, str]],
    threshold: float,
) -> Backoff:
    """잎부터 한 층씩 올라가며 확신도가 기준을 넘는 첫 층을 고른다."""
    layer = probabilities
    for level in range(len(parents) + 1):
        conf = confidence(layer)
        if conf >= threshold:
            return Backoff(argmax(layer), level, conf)
        if level < len(parents):
            layer = rollup(layer, parents[level])
    return Backoff(argmax(layer), -1, confidence(layer))


def max_gate(answers: dict[str, dict], ids: list[str], threshold: float) -> tuple[bool, str, float]:
    """항목 확률의 최댓값으로 판정한다. 평균은 확실한 신호 하나를 묻는다."""
    worst = max(ids, key=lambda qid: answers[qid]["noul"])
    value = answers[worst]["noul"]
    return value >= threshold, worst, value


def path_score(edge_probabilities: list[float]) -> float:
    """간선 확률의 기하평균. 깊이가 다른 잎을 공정하게 비교한다."""
    logs = [math.log(max(p, 1e-12)) for p in edge_probabilities]
    return math.exp(sum(logs) / len(logs))


# --- 호출 -----------------------------------------------------------------


class DryRun(Exception):
    """키가 없을 때 첫 요청 계획을 보여 주고 사례를 멈춘다."""


class Judge:
    def __init__(self, model: str, dry_run: bool, cache_path: Path | None) -> None:
        self.model = model
        self.key = os.environ.get(KEY_ENV)
        self.dry_run = dry_run or not self.key
        self.cache_path = cache_path
        self.cache: dict[str, Any] = {}
        if cache_path and cache_path.exists():
            self.cache = json.loads(cache_path.read_text())

    def ask(self, state: Any, questions: dict[str, dict]) -> tuple[dict, dict, float, bool]:
        body = {"state": state, "model": self.model, "questions": questions}
        digest = hashlib.sha256(json.dumps(body, sort_keys=True).encode()).hexdigest()
        if digest in self.cache:
            hit = self.cache[digest]
            return hit["answers"], hit["usage"], hit["latency_ms"], True
        if self.dry_run:
            raise DryRun(_plan(body))
        started = time.perf_counter()
        response = _post(body, self.key)
        latency_ms = (time.perf_counter() - started) * 1000
        self.cache[digest] = {
            "answers": response["answers"],
            "usage": response["usage"],
            "latency_ms": latency_ms,
            "model": response.get("model"),
        }
        if self.cache_path:
            self.cache_path.write_text(json.dumps(self.cache, ensure_ascii=False, indent=1))
        return response["answers"], response["usage"], latency_ms, False


def _post(body: dict, key: str, attempts: int = 4) -> dict:
    data = json.dumps(body).encode()
    for attempt in range(attempts):
        request = urllib.request.Request(
            API_URL,
            data=data,
            headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return json.loads(response.read())
        except urllib.error.HTTPError as error:
            # 429와 529만 다시 보낸다. 나머지는 요청 자체가 틀렸다.
            if error.code in (429, 529) and attempt < attempts - 1:
                time.sleep(2**attempt)
                continue
            detail = error.read().decode(errors="replace")[:500]
            raise RuntimeError(f"TypeSafe {error.code}: {detail}") from None
    raise RuntimeError("unreachable")


def _plan(body: dict) -> str:
    state_chars = len(json.dumps(body["state"], ensure_ascii=False))
    kinds: dict[str, int] = {}
    for question in body["questions"].values():
        kinds[question["type"]] = kinds.get(question["type"], 0) + 1
    summary = ", ".join(f"{k} {v}" for k, v in sorted(kinds.items()))
    ids = list(body["questions"])
    shown = ", ".join(ids[:8]) + (f", … +{len(ids) - 8}" if len(ids) > 8 else "")
    return f"state ~{state_chars // 4} tokens, questions {len(ids)} ({summary}): {shown}"


# --- 사슬과 기록 -------------------------------------------------------------


@dataclass
class Chain:
    judge: Judge
    demo: str
    case_id: str
    max_steps: int
    records: list[dict] = field(default_factory=list)
    chain_id: str = field(default_factory=lambda: uuid.uuid4().hex[:12])
    steps: int = 0

    def ask(
        self,
        state: Any,
        questions: dict[str, dict],
        *,
        trigger: str,
        parent: str | None = None,
    ) -> tuple[str, dict]:
        """한 단을 호출하고 call_id와 답을 돌려준다."""
        if self.steps >= self.max_steps:
            raise RuntimeError(f"chain depth {self.max_steps} exceeded at {trigger}")
        self.steps += 1
        answers, usage, latency_ms, cached = self.judge.ask(state, questions)
        call_id = f"{self.chain_id}:{self.steps}"
        self.records.append(
            {
                "kind": "call",
                "demo": self.demo,
                "case": self.case_id,
                "chain_id": self.chain_id,
                "call_id": call_id,
                "step": self.steps,
                "parent_call_id": parent,
                "trigger": trigger,
                "questions": list(questions),
                "answers": answers,
                "input_tokens": usage.get("input_tokens", 0),
                "latency_ms": round(latency_ms, 1),
                "cached": cached,
            }
        )
        _print_step(self.steps, trigger, questions, answers, latency_ms, cached)
        return call_id, answers

    def decide(
        self,
        name: str,
        *,
        single_shot: Any,
        applied: Any,
        applied_step: int,
        expected: Any = None,
        backoff_level: int = 0,
        note: str = "",
    ) -> None:
        """한 결정의 단발 답(1단만 썼을 때)과 사슬 답을 함께 남긴다."""
        self.records.append(
            {
                "kind": "decision",
                "demo": self.demo,
                "case": self.case_id,
                "chain_id": self.chain_id,
                "decision": name,
                "single_shot": single_shot,
                "applied": applied,
                "applied_step": applied_step,
                "backoff_level": backoff_level,
                "expected": expected,
                "note": note,
            }
        )
        mark = "" if expected is None else (" ok" if applied == expected else f" MISS(expected {expected})")
        changed = "" if single_shot == applied else f"  [1단: {single_shot}]"
        print(f"    => {name}: {applied} (step {applied_step}, backoff {backoff_level}){changed}{mark}")
        if note:
            print(f"       {note}")


def _print_step(step: int, trigger: str, questions: dict, answers: dict, latency_ms: float, cached: bool) -> None:
    source = "cache" if cached else f"{latency_ms:.0f} ms"
    print(f"  step {step} [{trigger}] {len(questions)} questions, {source}")
    for qid, answer in list(answers.items())[:12]:
        print(f"    {qid:<38} {_brief(answer)}")
    if len(answers) > 12:
        print(f"    … {len(answers) - 12} more")


def _brief(answer: dict) -> str:
    if answer["type"] == "noul":
        return f"P(yes)={answer['noul']:.2f}"
    if answer["type"] == "choice":
        top = sorted(answer["probabilities"].items(), key=lambda kv: -kv[1])[:3]
        dist = " ".join(f"{k}:{v:.2f}" for k, v in top)
        return f"{answer['choice']} conf={answer['confidence']:.2f} [{dist}]"
    return f"score={answer['score']:.2f} conf={answer['confidence']:.2f}"


# --- 데모 실행 -------------------------------------------------------------


def run_demo(demo: str, cases: list[dict], run_case, max_steps: int) -> None:
    parser = argparse.ArgumentParser(description=f"judge chain demo: {demo}")
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--dry-run", action="store_true", help="키가 있어도 호출하지 않고 요청 계획만 본다")
    parser.add_argument("--case", help="이 id의 사례만 실행")
    args = parser.parse_args()

    RUNS_DIR.mkdir(exist_ok=True)
    judge = Judge(args.model, args.dry_run, RUNS_DIR / f"{demo}.cache.json")
    if judge.dry_run and not args.dry_run:
        print(f"{KEY_ENV}가 없어 요청 계획만 보입니다. 캐시에 있는 응답은 그대로 재생합니다.\n")

    records: list[dict] = []
    for case in cases:
        if args.case and case["id"] != args.case:
            continue
        print(f"\n== {demo} / {case['id']}: {case['title']}")
        chain = Chain(judge, demo, case["id"], max_steps)
        try:
            run_case(chain, case)
        except DryRun as plan:
            print(f"  step {chain.steps + 1} plan: {plan}")
            print("  (다음 단은 이 단의 답에 따라 정해지므로 계획을 여기서 멈춥니다)")
        records.extend(chain.records)

    if records:
        out = RUNS_DIR / f"{demo}.jsonl"
        out.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in records))
        print(f"\n기록: {out.relative_to(HERE)}  (근거 요약: python3 report.py)")


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    sys.exit(1)
