#!/usr/bin/env python3
"""raw의 회차 행에서 이슈별 지표를 계산해 processed/metrics.csv 한 표로 만든다. 판정 기준은 design.md에 있다."""
from __future__ import annotations

import csv
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
USAGE_FIELDS = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens", "output_tokens"]
MODEL_FIELDS = {"input_tokens": "inputTokens", "cache_creation_input_tokens": "cacheCreationInputTokens",
                "cache_read_input_tokens": "cacheReadInputTokens", "output_tokens": "outputTokens"}
SHORT = {"input_tokens": "in", "cache_creation_input_tokens": "cc", "cache_read_input_tokens": "cr",
         "output_tokens": "out"}


def parent_of(event: dict):
    return (event.get("extra") or {}).get("parent_tool_use_id")


def blocks(event: dict, kind: str):
    return [b for b in event.get("blocks", []) if b.get("type") == kind]


def segments(events: list[dict]) -> list[list[dict]]:
    out, cur = [], []
    for e in events:
        cur.append(e)
        if e.get("type") == "result":
            out.append(cur)
            cur = []
    return out


def message_sums(segment: list[dict]) -> tuple[dict, dict]:
    """메시지 id마다 마지막 usage 하나만 센다. 부모 없는 메시지(main)와 있는 메시지(sub)로 나눈다."""
    last: dict[str, tuple[bool, dict]] = {}
    for e in segment:
        if e.get("type") == "assistant" and e.get("message_id") and e.get("usage"):
            last[e["message_id"]] = (parent_of(e) is not None, e["usage"])
    main = {f: 0 for f in USAGE_FIELDS}
    sub = {f: 0 for f in USAGE_FIELDS}
    for is_sub, usage in last.values():
        target = sub if is_sub else main
        for f in USAGE_FIELDS:
            target[f] += int(usage.get(f) or 0)
    return main, sub


def add(a: dict, b: dict) -> dict:
    return {f: a[f] + b[f] for f in USAGE_FIELDS}


def equal(usage: dict, total: dict) -> bool:
    return all(int(usage.get(f) or 0) == total[f] for f in USAGE_FIELDS)


def text_of(value) -> str:
    return json.dumps(value, ensure_ascii=False).lower()


def metrics(row: dict) -> dict:
    obs, events, requests = row["obs"], row["events"], row["requests"]
    out: dict = {"run_id": row["run_id"], "condition": row["condition"], "trial_id": row["trial_id"],
                 "calls": len(row["calls"]), "results": obs["results"], "turns_sent": obs["turns_sent"],
                 "marker_final": obs["marker_final"], "marker_at": obs["marker_at"], "trigger_at": obs["trigger_at"],
                 "stop_at": obs["stop_at"], "exit_at": obs["exit_at"], "exit_code": obs["exit_code"],
                 "notes": "|".join(obs["notes"])}
    results = [e for e in events if e.get("type") == "result"]
    uses = [(b, e) for e in events if e.get("type") == "assistant" for b in blocks(e, "tool_use")]
    agent_uses = [b for b, e in uses if b["name"] in ("Agent", "Task") and parent_of(e) is None]
    agent_id = agent_uses[0]["id"] if agent_uses else None
    # 하위 에이전트 이벤트(17)
    sub_events = [e for e in events if e.get("type") in ("assistant", "user") and parent_of(e) is not None]
    sub_ids = {parent_of(e) for e in sub_events}
    out["agent_uses"] = len(agent_uses)
    out["sub_event_count"] = len(sub_events)
    out["sub_parent_match"] = bool(sub_events) and sub_ids == {agent_id}
    agent_result = [e for e in events if e.get("type") == "user" and parent_of(e) is None
                    and any(b.get("tool_use_id") == agent_id for b in blocks(e, "tool_result"))]
    out["agent_result_at"] = agent_result[0]["at"] if agent_result else None
    out["last_sub_event_at"] = max((e["at"] for e in sub_events), default=None)
    out["agent_result_after_last_sub"] = (out["agent_result_at"] is not None and out["last_sub_event_at"] is not None
                                          and out["agent_result_at"] >= out["last_sub_event_at"])
    task_events = [e for e in events if e.get("type") == "system" and str(e.get("subtype", "")).startswith("task")]
    out["task_subtypes"] = "|".join(sorted({e["subtype"] for e in task_events}))
    out["task_start_match"] = any(e.get("subtype") == "task_started" and e.get("tool_use_id") == agent_id
                                  for e in task_events)
    out["task_end_match"] = any(e.get("subtype") in ("task_notification", "task_updated")
                                and e.get("tool_use_id") == agent_id
                                and str(e.get("status", "")) in ("completed", "stopped", "failed", "killed")
                                for e in task_events)
    out["first_result_at"] = results[0]["at"] if results else None
    out["second_result_at"] = results[1]["at"] if len(results) > 1 else None
    marker_at = obs["marker_at"]
    out["agent_result_5s_before_marker"] = (out["agent_result_at"] is not None and marker_at is not None
                                            and out["agent_result_at"] + 5 <= marker_at)
    out["first_result_before_marker"] = (out["first_result_at"] is not None and marker_at is not None
                                         and out["first_result_at"] < marker_at)
    out["second_result_after_marker"] = (out["second_result_at"] is not None and marker_at is not None
                                         and out["second_result_at"] > marker_at and obs["turns_sent"] == 1)
    # 사용량(19)
    segs = segments(events)
    cum_main = {f: 0 for f in USAGE_FIELDS}
    cum_tree = {f: 0 for f in USAGE_FIELDS}
    for k, seg in enumerate(segs[:2], start=1):
        result = seg[-1]
        main, sub = message_sums(seg)
        cum_main, cum_tree = add(cum_main, main), add(cum_tree, add(main, sub))
        usage = result.get("usage") or {}
        for f in USAGE_FIELDS:
            out[f"r{k}_usage_{SHORT[f]}"] = usage.get(f)
            out[f"r{k}_main_{SHORT[f]}"] = main[f]
            out[f"r{k}_sub_{SHORT[f]}"] = sub[f]
        out[f"r{k}_eq_main"] = equal(usage, main)
        out[f"r{k}_eq_tree"] = equal(usage, add(main, sub))
        out[f"r{k}_eq_cum_main"] = equal(usage, cum_main)
        out[f"r{k}_eq_cum_tree"] = equal(usage, cum_tree)
        model_usage = result.get("modelUsage") or {}
        totals = {f: sum(int((m or {}).get(MODEL_FIELDS[f]) or 0) for m in model_usage.values()) for f in USAGE_FIELDS}
        out[f"r{k}_models"] = len(model_usage)
        out[f"r{k}_modelusage_eq_usage"] = bool(model_usage) and equal(usage, totals)
        out[f"r{k}_cost"] = result.get("total_cost_usd")
    out["json_result_has_usage"] = bool(results) and bool(results[0].get("usage"))
    # 훅과 키 저장소(23, 3)
    hook_rows = obs["hook_rows"]
    out["hook_rows"] = len(hook_rows)
    out["hook_agent_rows"] = sum(1 for r in hook_rows if r.get("agent_id"))
    out["hook_bash_agent_denied"] = sum(1 for r in hook_rows if r.get("tool_name") == "Bash" and r.get("agent_id")
                                        and "security" in str(r.get("command")) and r.get("denied"))
    out["hook_bash_main_denied"] = sum(1 for r in hook_rows if r.get("tool_name") == "Bash" and not r.get("agent_id")
                                       and "security" in str(r.get("command")) and r.get("denied"))
    out["hook_keys"] = "|".join(sorted({k for r in hook_rows for k in r.get("keys", [])}))
    out["security_requests"] = sum(1 for r in requests if "security" in str((r.get("input") or {}).get("command", "")))
    out["security_attempted"] = any(b["name"] == "Bash" and "security" in str((b.get("input") or {}).get("command", ""))
                                    for b, e in uses)
    out["leak_tool_result"] = obs["leak_tool_result"]
    out["leak_assistant"] = obs["leak_assistant"]
    out["any_tool_result_error"] = any(b.get("is_error") for e in events if e.get("type") == "user"
                                       for b in blocks(e, "tool_result"))
    out["nested_created"] = any(b["name"] in ("Agent", "Task") and parent_of(e) is not None for b, e in uses) or any(
        r.get("tool_name") in ("Agent", "Task") and r.get("agent_id") for r in hook_rows)
    # 슬래시 명령(26)
    out["touch_requests"] = sum(1 for r in requests if r["tool_name"] == "Bash" and "touch" in str(r["input"]))
    out["requests"] = len(requests)
    first = results[0] if results else {}
    out["result_is_error"] = first.get("is_error")
    out["result_text_len"] = len(str(first.get("result") or ""))
    out["result_text_has_unknown"] = "unknown" in str(first.get("result") or "").lower()
    out["compact_boundary"] = any(e.get("type") == "system" and e.get("subtype") == "compact_boundary" for e in events)
    # 적용 설정(4)
    inits = [e for e in events if e.get("type") == "system" and e.get("subtype") == "init"]
    out["init_count"] = len(inits)
    for k in (1, 2):
        out[f"init{k}_permission"] = (inits[k - 1]["fields"].get("permissionMode") if len(inits) >= k else None)
    out["init_keys"] = "|".join(sorted(inits[0]["fields"].keys())) if inits else ""
    out["sandbox_in_init"] = any(("sandbox" in text_of(i["fields"]) or "network" in text_of(i["fields"])) for i in inits)
    ctrl = [e for e in events if e.get("type") == "control_response" and
            str(e.get("request_id", "")).startswith("exp-") and e.get("request_id") not in ("exp-interrupt",)]
    out["sandbox_in_control"] = any(("sandbox" in text_of(c["body"]) or "network" in text_of(c["body"])) for c in ctrl)
    out["set_mode_ok"] = any(e.get("request_id") == "exp-set_mode" and e.get("subtype") == "success"
                             for e in events if e.get("type") == "control_response")
    out["interrupt_ok"] = any(e.get("request_id") == "exp-interrupt" and e.get("subtype") == "success"
                              for e in events if e.get("type") == "control_response")
    # 멈춤(18)
    snaps = obs.get("snapshots") or {}
    out["descendants_at_stop"] = len(snaps.get("at_stop", []))
    out["alive_3s_after_stop"] = len(snaps.get("alive_3s_after_stop", []))
    out["group_members_at_end"] = len(snaps.get("group_members_at_end", []))
    out["system_subtypes"] = "|".join(sorted({f"{e['type']}/{e.get('subtype')}" for e in events
                                              if e.get("type") in ("system", "rate_limit_event", "stream_event")}))
    return out


def main() -> int:
    rows = []
    for path in sorted((ROOT / "data" / "raw").glob("claude-*.jsonl")):
        for line in path.read_text(encoding="utf-8").splitlines():
            row = json.loads(line)
            if row.get("kind") == "trial":
                rows.append(metrics(row))
    out = ROOT / "data" / "processed" / "metrics.csv"
    out.parent.mkdir(parents=True, exist_ok=True)
    fields = list(rows[0].keys()) if rows else []
    for row in rows:
        for key in row:
            if key not in fields:
                fields.append(key)
    with out.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        for row in rows:
            writer.writerow({k: ("" if v is None else v) for k, v in row.items()})
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
