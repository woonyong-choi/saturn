"""표본, 조건, 입력 대본. 수집 전에 design.md와 함께 고정한다. 수집 뒤에 바꾸지 않는다."""

from __future__ import annotations

import random

PLAN_VERSION = 1
ARMS = ("none", "provider", "rrf", "jev")
# claude-codex: Claude에서 쌓고 경계에서 Codex로 전환한다. 새 session 전달 두 방식과 provider 요약 전달만 비교한다(조건 `none`은 정의되지 않는다).
CROSS = "claude-codex"
CROSS_ARMS = ("provider", "rrf", "jev")
CROSS_SEEDS = tuple(range(1, 9))
CROSS_MODEL_LABEL = "GPT-5.6-Luna"  # /model codex 목록에서 이 이름이 든 줄을 고르고, 고른 뒤 화면으로 확인한다
PROVIDERS = {
    "claude": {"model": "claude/haiku", "t_abs": 20000},
    "codex": {"model": "codex/gpt-5.6-luna", "t_abs": 20000},
    CROSS: {"model": "claude/haiku", "t_abs": 20000},
    # 2단계: 1단계에서 Codex 경계 맥락이 20000 아래인 시도가 많아 새 session이 열리지 않았다. 기준만 16000으로 낮춰 새 프로젝트로 다시 잰다.
    "codex-t16": {"model": "codex/gpt-5.6-luna", "t_abs": 16000, "id": "codex"},
}
STAGE2 = "codex-t16"
STAGE2_SEEDS = tuple(range(21, 33))

def engine_id(provider: str) -> str:
    return PROVIDERS[provider].get("id", provider)
FORMAL_SEEDS = tuple(range(1, 13))
PILOT_SEEDS = (9001, 9002)
ORDER_SEED = 7007
COMMON_OVERRIDES = ["permission.mode=full", "model.mode=manual"]

# 가중치: 입력 1, 캐시 쓰기 w(provider 설명자), 캐시 읽기 0.1, 출력 5. 출력 4와 8도 민감도로 보고한다.
CACHE_WRITE = {"claude": 1.25, "codex": 1.0}
CACHE_READ = 0.1
OUTPUT_WEIGHT = 5.0

SETUP_TURNS = [
    "이 저장소 구조를 훑고 `python3 -m unittest -q`로 테스트 상태를 알려줘. 파일은 수정하지 마.",
    "`sh tools/incident.sh`를 실행해서 출력을 끝까지 읽어. 이 피드는 한 번만 읽을 수 있어서 다시 실행하면 비어 있어. 아직 아무것도 고치지 말고 `읽음` 한 단어로만 답해.",
    "요청 추적 헤더 이름은 앞으로 `X-Req-Id`로 쓴다. 이건 앞으로도 계속 지킬 규칙이야. `확인`이라고만 답해.",
    "README.md의 설정 절을 읽고 `읽음`이라고만 답해.",
    "`sh tools/dump-metrics.sh`를 실행하고 결과를 읽어. `읽음`이라고만 답해.",
    "정정할게. 요청 추적 헤더 이름은 `{header_new}`로 쓴다. 앞서 말한 `X-Req-Id` 규칙은 취소하고 이 이름을 앞으로 계속 지켜. `확인`이라고만 답해.",
    "app/client.py와 app/store.py를 읽고 `읽음`이라고만 답해.",
]
BOUNDARY_TURN = "docs/architecture.md를 읽고 `읽음`이라고만 답해."
FOLLOW_UPS = [
    ("f1", "아까 incident 피드에서 확인한 원인대로 코드를 고쳐줘. 요청 헤더 이름은 우리가 정정해서 정한 이름을 쓰고, 타임아웃 값은 피드가 요구한 값으로 맞춰줘. 끝나면 `python3 -m unittest -q`를 돌려줘."),
    ("f2", "그 incident의 티켓 번호가 뭐였어? 번호만 답해."),
    ("f3", "dump-metrics 결과에서 p99가 가장 큰 엔드포인트를 `app/report.py`의 SLOW_ENDPOINT 값으로 넣어줘. 끝나면 `python3 -m unittest -q`를 돌려줘."),
]

CALL_CAPS = {"claude": 700, "codex": 700, CROSS: 400, STAGE2: 700, "router": 1800}


def trial_list(provider: str, seeds: tuple[int, ...]) -> list[tuple[int, str]]:
    """(시드, 조건) 목록. 순서는 고정 시드로 섞는다."""
    rng = random.Random(ORDER_SEED + sum(map(ord, provider)))
    pairs = [(s, a) for s in seeds for a in (CROSS_ARMS if provider == CROSS else ARMS)]
    rng.shuffle(pairs)
    return pairs


def boundary_overrides(provider: str, arm: str) -> list[str]:
    if arm == "provider":
        return ["context.mode=provider"]
    if arm in ("rrf", "jev"):
        return [f"provider.{engine_id(provider)}.context.t_abs={PROVIDERS[provider]['t_abs']}", f"context.select.packet={arm}"]
    return []


def base_overrides(provider: str, arm: str) -> list[str]:
    extra = ["context.mode=provider"] if arm == "provider" else []
    return [*COMMON_OVERRIDES, f"model.default={PROVIDERS[provider]['model']}", *extra]


def cross_overrides(arm: str) -> list[str]:
    """Claude에서 Codex로 전환할 때 TUI 접속에 거는 설정. 전환 패킷은 받는 쪽 Codex의 기준으로 만든다."""
    if arm == "provider":
        return ["context.mode=provider", f"provider.codex.context.t_abs={PROVIDERS['codex']['t_abs']}"]
    return [f"provider.codex.context.t_abs={PROVIDERS['codex']['t_abs']}", f"context.select.packet={arm}"]
