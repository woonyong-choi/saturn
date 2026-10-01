"""빠른 조정 규칙을 합성 판단에 흘려 100건 묶음 기록을 raw/에 쓴다.

규칙은 saturn-terminal/core/src/judges/calibration.rs의 ThresholdState.observe와
ask_probability를 그대로 옮긴 것이다.
"""

from __future__ import annotations

import argparse
import json
import logging
import math
import sys
from collections import deque
from pathlib import Path

import numpy as np

logger = logging.getLogger(__name__)

SEED = 120
REPLICATES = 100
JUDGMENTS = 10_000
BLOCK = 100
SWITCH_AT = 5_000

CENTER = 0.8
BOUNDS = (0.5, 0.95)
TARGET_WRONG_RATE = 0.05

# calibration.rs 상수
FAST_RANGE = 0.05
FAST_STEP = 0.01
STABLE_WINDOW = 100
STABLE_BAND = 0.01
RATE_SHIFT = 0.25
FAST_RATE_WEIGHT = 0.2
SLOW_RATE_WEIGHT = 0.02
RATE_SHIFT_MIN_SIGNALS = 20
MAX_ASK_RATE = 0.05
MIN_ASK = 0.01
MAX_ASK = 0.1
ASK_WIDTH = 0.1

# 합성 Jev 확률 분포: 0.7 × Beta(5, 1.2) + 0.3 × Beta(1.2, 5)
MIX_YES = 0.7
BETA_YES = (5.0, 1.2)
BETA_NO = (1.2, 5.0)

# (조건, 신호 출처, 앞 절반 틀림 비율, 뒤 절반 틀림 비율)
CONDITIONS = (
    ("behavior-3", "behavior", 0.03, 0.03),
    ("behavior-5", "behavior", 0.05, 0.05),
    ("behavior-10", "behavior", 0.10, 0.10),
    ("behavior-shift", "behavior", 0.03, 0.10),
    ("asked-3", "asked", 0.03, 0.03),
    ("asked-5", "asked", 0.05, 0.05),
    ("asked-10", "asked", 0.10, 0.10),
)

WRONG = "wrong"
MISSED = "missed"
UNCONFIRMED = "unconfirmed"

_GRID = np.linspace(1e-6, 1.0 - 1e-6, 200_001)


class ThresholdState:
    """calibration.rs ThresholdState의 빠른 조정 부분."""

    def __init__(self, center: float, bounds: tuple[float, float]) -> None:
        self.center = center
        self.bounds = bounds
        self.fast_offset = 0.0
        self.target_wrong_rate = TARGET_WRONG_RATE
        self.signals = 0
        self.recent: deque[float] = deque()
        self.is_frozen = False
        self.wrong_rate: tuple[float, float] | None = None

    def current(self) -> float:
        return min(max(self.center + self.fast_offset, self.bounds[0]), self.bounds[1])

    def observe(self, signal: str, asked_with: float) -> bool:
        """신호 하나를 반영하고 급변으로 다시 시작했으면 참을 돌려준다."""
        if signal == UNCONFIRMED:
            return False
        is_wrong = signal == WRONG
        restarted = self._has_rate_shifted(is_wrong)
        if restarted:
            self.signals = 0
            self.recent = deque()
            self.is_frozen = False
        if self.is_frozen:
            return restarted
        step = FAST_STEP / math.sqrt(self.signals + 1)
        weight = 1.0 / max(asked_with, MIN_ASK)
        direction = 1.0 - self.target_wrong_rate if is_wrong else -self.target_wrong_rate
        offset = self.fast_offset + step * weight * direction
        self.fast_offset = min(max(offset, -FAST_RANGE), FAST_RANGE)
        self.signals += 1
        self._remember_current()
        return restarted

    def _has_rate_shifted(self, is_wrong: bool) -> bool:
        value = 1.0 if is_wrong else 0.0
        fast, slow = self.wrong_rate if self.wrong_rate is not None else (value, value)
        fast = fast + FAST_RATE_WEIGHT * (value - fast)
        slow = slow + SLOW_RATE_WEIGHT * (value - slow)
        self.wrong_rate = (fast, slow)
        return self.signals >= RATE_SHIFT_MIN_SIGNALS and abs(fast - slow) > RATE_SHIFT

    def _remember_current(self) -> None:
        self.recent.append(self.current())
        if len(self.recent) > STABLE_WINDOW:
            self.recent.popleft()
        if len(self.recent) < STABLE_WINDOW:
            return
        self.is_frozen = max(self.recent) - min(self.recent) <= 2.0 * STABLE_BAND


def ask_probability(
    probability: float, threshold: float, recent_asks: int, recent_judgments: int
) -> float:
    allowed = MAX_ASK_RATE * (recent_judgments + 1.0)
    if recent_asks + 1.0 > allowed:
        return MIN_ASK
    distance = abs(probability - threshold)
    return MIN_ASK + (MAX_ASK - MIN_ASK) * math.exp(-distance / ASK_WIDTH)


def correct_probability(probability: np.ndarray, delta: float) -> np.ndarray:
    """보정된 Jev 확률을 사용자 차이 delta만큼 logit에서 옮긴 실제 정답 확률."""
    logit = np.log(probability / (1.0 - probability))
    return 1.0 / (1.0 + np.exp(-(logit - delta)))


def solve_delta(wrong_rate: float) -> float:
    """기준값 0.8에서 행동한 판단의 틀림 비율이 wrong_rate가 되는 delta."""
    low, high = -6.0, 6.0
    for _ in range(100):
        middle = (low + high) / 2.0
        if _acted_wrong_rate(CENTER, middle) < wrong_rate:
            low = middle
        else:
            high = middle
    return (low + high) / 2.0


def oracle_threshold(delta: float) -> float:
    """행동한 판단의 틀림 비율이 목표가 되는 가장 낮은 기준값."""
    low, high = 0.01, 0.99
    if _acted_wrong_rate(low, delta) <= TARGET_WRONG_RATE:
        return low
    for _ in range(100):
        middle = (low + high) / 2.0
        if _acted_wrong_rate(middle, delta) > TARGET_WRONG_RATE:
            low = middle
        else:
            high = middle
    return high


def balance_threshold(delta: float, signal_model: str) -> float:
    """틀림 신호 가중합 × (1 − α)와 놓침 신호 가중합 × α가 같아지는 기준값(묻기 상한 무시)."""
    low, high = 0.01, 0.99
    if _balance(low, delta, signal_model) <= 0.0:
        return low
    if _balance(high, delta, signal_model) >= 0.0:
        return high
    for _ in range(100):
        middle = (low + high) / 2.0
        if _balance(middle, delta, signal_model) > 0.0:
            low = middle
        else:
            high = middle
    return (low + high) / 2.0


def simulate(
    replicate: int, signal_model: str, delta_before: float, delta_after: float, judgments: int
) -> list[dict[str, float | int | bool]]:
    rng = np.random.default_rng([SEED, replicate])
    is_yes = rng.random(judgments) < MIX_YES
    yes = rng.beta(*BETA_YES, judgments)
    no = rng.beta(*BETA_NO, judgments)
    probability = np.clip(np.where(is_yes, yes, no), 1e-9, 1.0 - 1e-9)
    correct_draw = rng.random(judgments)
    ask_draw = rng.random(judgments)
    switch = min(SWITCH_AT, judgments // 2)
    correct_chance = np.concatenate(
        [
            correct_probability(probability[:switch], delta_before),
            correct_probability(probability[switch:], delta_after),
        ]
    )
    is_correct = correct_draw < correct_chance

    state = ThresholdState(CENTER, BOUNDS)
    asks = 0
    blocks: list[dict[str, float | int | bool]] = []
    block = _empty_block()
    for index in range(judgments):
        threshold = state.current()
        p = float(probability[index])
        q = ask_probability(p, threshold, asks, index)
        asked = bool(ask_draw[index] < q)
        asks += int(asked)
        acted = p >= threshold
        correct = bool(is_correct[index])
        signal = UNCONFIRMED
        if signal_model == "behavior" or asked:
            if acted and not correct:
                signal = WRONG
            elif not acted and correct:
                signal = MISSED
        restarted = state.observe(signal, q)

        block["acted"] += int(acted)
        block["acted_wrong"] += int(acted and not correct)
        block["skipped_correct"] += int(not acted and correct)
        block["wrong_signals"] += int(signal == WRONG)
        block["missed_signals"] += int(signal == MISSED)
        block["asks"] += int(asked)
        block["restarts"] += int(restarted)
        block["threshold_sum"] += threshold
        block["threshold_min"] = min(block["threshold_min"], threshold)
        block["threshold_max"] = max(block["threshold_max"], threshold)
        if (index + 1) % BLOCK == 0:
            block["block"] = index // BLOCK + 1
            block["frozen_end"] = state.is_frozen
            block["threshold_end"] = state.current()
            blocks.append(_finish_block(block))
            block = _empty_block()
    return blocks


def self_test() -> None:
    """calibration.rs 단위 테스트와 같은 성질을 확인한다."""
    state = ThresholdState(0.8, (0.5, 0.95))
    state.observe(WRONG, 1.0)
    first = state.fast_offset
    state.observe(WRONG, 1.0)
    if not (first > 0.0 and state.fast_offset - first < first):
        raise AssertionError("step must shrink with signals")

    state = ThresholdState(0.8, (0.5, 0.95))
    for _ in range(1_000):
        state.observe(WRONG, 0.0001)
    if abs(state.current() - (0.8 + FAST_RANGE)) > 1e-12:
        raise AssertionError("fast offset must stay within range")

    state = ThresholdState(0.8, (0.5, 0.95))
    for _ in range(STABLE_WINDOW):
        state.observe(MISSED, 1.0)
    frozen_at = state.fast_offset
    state.observe(MISSED, 1.0)
    if not (state.is_frozen and state.fast_offset == frozen_at):
        raise AssertionError("adaptation must stop after stable window")
    for _ in range(3):
        state.observe(WRONG, 1.0)
    if state.is_frozen or state.signals >= 10:
        raise AssertionError("rate shift must restart adaptation")

    if ask_probability(0.8, 0.8, 5, 100) != MIN_ASK:
        raise AssertionError("ask rate limit must use minimum")
    if not ask_probability(0.81, 0.8, 0, 100) > ask_probability(0.01, 0.8, 0, 100) > 0.0:
        raise AssertionError("ask probability must peak near threshold")


def main() -> int:
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--ts-utc", required=True)
    parser.add_argument("--replicates", type=int, default=REPLICATES)
    parser.add_argument("--judgments", type=int, default=JUDGMENTS)
    args = parser.parse_args()

    self_test()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    sim_path = args.out_dir / f"sim-{args.run_id}.jsonl"
    population_path = args.out_dir / f"population-{args.run_id}.jsonl"
    common = {"run_id": args.run_id, "ts_utc": args.ts_utc}

    with population_path.open("w", encoding="utf-8", newline="\n") as population_file:
        deltas = {}
        for condition, signal_model, rate_before, rate_after in CONDITIONS:
            delta_before = solve_delta(rate_before)
            delta_after = solve_delta(rate_after)
            deltas[condition] = (delta_before, delta_after)
            record = {
                **common,
                "trial_id": f"{condition}-population",
                "condition": condition,
                "signal_model": signal_model,
                "wrong_rate_before": rate_before,
                "wrong_rate_after": rate_after,
                "delta_before": round(delta_before, 6),
                "delta_after": round(delta_after, 6),
                "oracle_threshold_after": round(oracle_threshold(delta_after), 6),
                "balance_threshold_after": round(
                    balance_threshold(delta_after, signal_model), 6
                ),
                "wrong_rate_at_low_after": round(
                    _acted_wrong_rate(CENTER - FAST_RANGE, delta_after), 6
                ),
                "wrong_rate_at_high_after": round(
                    _acted_wrong_rate(CENTER + FAST_RANGE, delta_after), 6
                ),
            }
            population_file.write(json.dumps(record, sort_keys=True) + "\n")

    with sim_path.open("w", encoding="utf-8", newline="\n") as sim_file:
        for condition, signal_model, _, _ in CONDITIONS:
            delta_before, delta_after = deltas[condition]
            logger.info("simulating %s", condition)
            for replicate in range(args.replicates):
                blocks = simulate(
                    replicate, signal_model, delta_before, delta_after, args.judgments
                )
                for block in blocks:
                    record = {
                        **common,
                        "trial_id": f"{condition}-r{replicate:03d}-b{block['block']:03d}",
                        "condition": condition,
                        "signal_model": signal_model,
                        "replicate": replicate,
                        **block,
                    }
                    sim_file.write(json.dumps(record, sort_keys=True) + "\n")
    return 0


def _acted_wrong_rate(threshold: float, delta: float) -> float:
    density = _density()
    acted = _GRID >= threshold
    wrong = 1.0 - correct_probability(_GRID[acted], delta)
    return float((wrong * density[acted]).sum() / density[acted].sum())


def _balance(threshold: float, delta: float, signal_model: str) -> float:
    density = _density()
    correct = correct_probability(_GRID, delta)
    if signal_model == "behavior":
        q = MIN_ASK + (MAX_ASK - MIN_ASK) * np.exp(-np.abs(_GRID - threshold) / ASK_WIDTH)
        weight = 1.0 / q
    else:
        weight = np.ones_like(_GRID)
    acted = _GRID >= threshold
    wrong = ((1.0 - correct) * density * weight)[acted].sum()
    missed = (correct * density * weight)[~acted].sum()
    return float((1.0 - TARGET_WRONG_RATE) * wrong - TARGET_WRONG_RATE * missed)


def _density() -> np.ndarray:
    return MIX_YES * _beta_pdf(*BETA_YES) + (1.0 - MIX_YES) * _beta_pdf(*BETA_NO)


def _beta_pdf(a: float, b: float) -> np.ndarray:
    log_norm = math.lgamma(a) + math.lgamma(b) - math.lgamma(a + b)
    return np.exp((a - 1.0) * np.log(_GRID) + (b - 1.0) * np.log(1.0 - _GRID) - log_norm)


def _empty_block() -> dict[str, float | int | bool]:
    return {
        "acted": 0,
        "acted_wrong": 0,
        "skipped_correct": 0,
        "wrong_signals": 0,
        "missed_signals": 0,
        "asks": 0,
        "restarts": 0,
        "threshold_sum": 0.0,
        "threshold_min": math.inf,
        "threshold_max": -math.inf,
    }


def _finish_block(block: dict[str, float | int | bool]) -> dict[str, float | int | bool]:
    finished = dict(block)
    finished["threshold_mean"] = round(float(finished.pop("threshold_sum")) / BLOCK, 6)
    for key in ("threshold_min", "threshold_max", "threshold_end"):
        finished[key] = round(float(finished[key]), 6)
    return finished


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        logger.exception("collect failed")
        sys.exit(1)
