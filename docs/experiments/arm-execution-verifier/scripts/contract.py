"""공통 trial 계약과 사용량 합산. 오프라인 실행과 온라인 실행 입력이 같은 검사를 쓴다."""

from __future__ import annotations

ARMS = ("full", "code", "llm", "jev")
PRESERVE_ARMS = ("F", "R", "J", "N")  # 보존 우선 비교의 조건. 선별 비교의 ARMS와 섞지 않는다
STATUSES = ("ok", "failed", "delivery_unknown", "incomplete")
ROLES = ("parent", "child", "jev", "selector")
USAGE_FIELDS = (
    "input_tokens",
    "cache_creation_input_tokens",
    "cache_read_input_tokens",
    "output_tokens",
)
TRIAL_FIELDS = (
    "task_id",
    "family_id",
    "split",
    "fixture_hash",
    "arm",
    "seed",
    "provider",
    "model",
    "model_version",
    "policy_version",
    "input_id",
    "run_id",
    "session_id",
    "decision_id",
    "started_at",
    "ended_at",
    "latency_s",
    "raw_files",
    "raw_sha256",
    "selection",
    "answer",
    "check",
    "status",
    "usage",
)
# 오프라인에서는 engine이 없어 null이다. 온라인 입력은 모두 값이 있어야 한다.
ONLINE_REQUIRED = ("input_id", "run_id", "session_id", "model_version")


class ContractError(RuntimeError):
    pass


def _fail(message: str) -> None:
    raise ContractError(message)


def validate_trial(trial: dict, online: bool = False, arms: tuple = ARMS) -> None:
    for key in TRIAL_FIELDS:
        if key not in trial:
            _fail(f"trial field missing: {key} ({trial.get('task_id')}/{trial.get('arm')})")
    if trial["arm"] not in arms:
        _fail("unknown arm: " + str(trial["arm"]))
    if trial["status"] not in STATUSES:
        _fail("unknown status: " + str(trial["status"]))
    if online:
        for key in ONLINE_REQUIRED:
            if trial[key] is None:
                _fail("online trial needs " + key)
    if trial["status"] == "ok" and trial["started_at"] is None:
        _fail("ok trial without start time")


def validate_usage(entries: object) -> list[dict]:
    """항목마다 모든 필드가 있어야 한다. 미보고는 값 null이지 키 누락이 아니다."""
    if not isinstance(entries, list):
        _fail("usage is not a list")
    seen: dict[str, dict] = {}
    for e in entries:
        if not isinstance(e, dict):
            _fail("usage entry is not an object")
        for key in ("call_id", "role", "parent_call_id", "includes_children", *USAGE_FIELDS):
            if key not in e:
                _fail("usage field missing: " + key)
        if e["role"] not in ROLES:
            _fail("unknown usage role: " + str(e["role"]))
        if e["call_id"] in seen:
            _fail("duplicate usage for call: " + str(e["call_id"]))
        for key in USAGE_FIELDS:
            v = e[key]
            if v is not None and (isinstance(v, bool) or not isinstance(v, int) or v < 0):
                _fail("invalid usage value: " + key)
        seen[e["call_id"]] = e
    for e in entries:
        if e["role"] != "child":
            continue
        parent = seen.get(e["parent_call_id"])
        if parent is None or parent["role"] == "child":
            _fail("child usage without parent call: " + str(e["call_id"]))
        if parent["includes_children"]:
            _fail("child usage counted twice: " + str(e["call_id"]))
    return entries


def merge_usage(entries: list[dict]) -> dict:
    """parent, child, 선택 호출을 호출 ID별로 한 번씩 합친다. 하나라도 미보고면 그 필드의 합은 null."""
    validate_usage(entries)
    total: dict = {}
    for key in USAGE_FIELDS:
        values = [e[key] for e in entries]
        total[key] = None if not values or any(v is None for v in values) else sum(values)
    total["known"] = {
        key: sum(e[key] for e in entries if e[key] is not None) for key in USAGE_FIELDS
    }
    total["unreported_calls"] = sum(
        any(e[key] is None for key in USAGE_FIELDS) for e in entries
    )
    total["calls"] = len(entries)
    return total


def check_cross_trial_usage(trials: list[dict]) -> None:
    """같은 호출의 사용량이 서로 다른 trial에 두 번 들어가지 않는다."""
    seen: set[str] = set()
    for t in trials:
        for e in t["usage"]:
            if e["call_id"] in seen:
                _fail("usage call shared by trials: " + str(e["call_id"]))
            seen.add(e["call_id"])
