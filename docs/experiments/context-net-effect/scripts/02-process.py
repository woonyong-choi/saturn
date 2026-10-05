"""원자료(data/raw/*.json.gz)에서 trial 표를 다시 만든다. 같은 입력이면 같은 바이트를 낸다.

사용: python3 02-process.py [--run formal|pilot] [--check]
`--check`는 결과를 다시 만들어 기존 파일과 바이트가 같은지만 본다.
"""

from __future__ import annotations

import argparse
import gzip
import json
import sys
from pathlib import Path

import plan

HERE = Path(__file__).resolve().parent
EXP = HERE.parent
RAW = EXP / "data" / "raw"
OUT = EXP / "results" / "tables"


def weighted(provider: str, row: dict, output_weight: float = plan.OUTPUT_WEIGHT) -> float:
    return (
        row["input"]
        + plan.CACHE_WRITE[provider] * row["cache_write"]
        + plan.CACHE_READ * row["cache_read"]
        + output_weight * row["output"]
    )


def cost(by_provider: dict[str, dict], output_weight: float = plan.OUTPUT_WEIGHT) -> float:
    return sum(weighted(p, u, output_weight) for p, u in by_provider.items())


def zero() -> dict:
    return {"input": 0, "cache_read": 0, "cache_write": 0, "output": 0}


def window_usage(store: dict, window_runs: set[int]) -> dict[str, dict]:
    """경계 뒤 실행의 사용량을 provider별로. Claude는 턴 값의 합, Codex는 thread 누적값의 구간 차이."""
    session_provider = {s["id"]: s["provider"] for s in store["sessions"]}
    totals: dict[str, dict] = {}
    by_session: dict[int, list[dict]] = {}
    for r in store["usage"]:
        by_session.setdefault(r["session_id"], []).append(r)
    for session_id, session_rows in by_session.items():
        provider = session_provider[session_id]
        total = totals.setdefault(provider, zero())
        if provider == "claude":
            for r in session_rows:
                if r["run_id"] in window_runs:
                    total["input"] += r["input_tokens"] or 0
                    total["cache_read"] += r["cache_read_tokens"] or 0
                    total["cache_write"] += r["cache_write_tokens"] or 0
                    total["output"] += r["output_tokens"] or 0
            continue
        before = zero()
        after = None
        for r in session_rows:
            cur = {
                "input": r["input_tokens"] or 0,
                "cache_read": r["cache_read_tokens"] or 0,
                "cache_write": r["cache_write_tokens"] or 0,
                "output": r["output_tokens"] or 0,
            }
            if r["run_id"] in window_runs:
                after = cur
            elif after is None:
                before = cur
        if after is not None:
            for key in total:
                total[key] += max(after[key] - before[key], 0)
    return totals


def event_bodies(store: dict) -> list[dict]:
    return [dict(seq=e["seq"], run_id=e["run_id"], body=json.loads(e["body"])) for e in store["events"]]


def find_evidence(events: list[dict], needle: str) -> tuple[int | None, int | None]:
    """명령줄에 needle이 든 도구 호출 번호와 그 결과 번호. 첫 호출을 쓴다."""
    call_seq = call_id = None
    for e in events:
        call = e["body"].get("ToolCall")
        if call and needle in json.dumps(call.get("activity", ""), ensure_ascii=False):
            call_seq, call_id = e["seq"], call["call_id"]
            break
    if call_seq is None:
        return None, None
    for e in events:
        res = e["body"].get("ToolResult")
        if res and res["call_id"] == call_id:
            return call_seq, e["seq"]
    return call_seq, None


def process_trial(res: dict) -> dict:
    provider, arm = res["provider"], res["arm"]
    facts, store = res["facts"], res["store"]
    turns = {t["label"]: t for t in res["turns"]}
    inputs = sorted(store["inputs"], key=lambda r: r["id"])
    labels = [t["label"] for t in res["turns"] if t["label"] != "switch"]
    input_of = {label: inputs[i]["id"] for i, label in enumerate(labels) if i < len(inputs)}
    runs = store["runs_meta"]
    row: dict = {
        "trial": res["trial"], "provider": provider, "seed": res["seed"], "arm": arm,
        "complete": res["complete"], "turn_statuses": ",".join(f"{t['label']}:{t['status']}" for t in res["turns"]),
        "provider_requests": len(res["turns"]),
    }
    boundary_input = input_of.get("boundary")
    b_runs = [r for r in runs if r["input_id"] == boundary_input]
    if not b_runs or not any(r["ended_at"] for r in b_runs):
        row.update(usable=False)  # 경계 전에 멈춘 trial은 인프라 실패로 따로 센다
        return row
    boundary_end = max(r["ended_at"] or 0 for r in b_runs)
    window_runs = {r["id"] for r in runs if r["started_at"] >= boundary_end - 1}
    window_inputs = {input_of[x] for x in input_of if x in ("f1", "f2", "f3", "compact")}
    by_provider = window_usage(store, window_runs)
    usage = {k: sum(u[k] for u in by_provider.values()) for k in zero()}
    judg = [j for j in store["judgments"] if j["input_id"] is None or j["input_id"] in window_inputs]
    compact = [j for j in store["judgments"] if j["input_id"] is None]
    router_tokens = sum((j["input_tokens"] or 0) + (j["output_tokens"] or 0) for j in judg)
    router_compact_tokens = sum((j["input_tokens"] or 0) + (j["output_tokens"] or 0) for j in compact)
    last_end = max([r["ended_at"] or 0 for r in runs if r["id"] in window_runs] + [boundary_end])
    events = event_bodies(store)
    win_events = [e for e in events if e["run_id"] in window_runs]
    tool_calls = [e["body"]["ToolCall"] for e in win_events if "ToolCall" in e["body"]]
    def runs_script(call: dict, name: str) -> bool:
        text = json.dumps(call.get("activity", ""), ensure_ascii=False)
        return f"sh tools/{name}" in text or f"./tools/{name}" in text

    reruns_metrics = sum(runs_script(c, "dump-metrics.sh") for c in tool_calls)
    reruns_incident = sum(runs_script(c, "incident.sh") for c in tool_calls)
    inc_call, _ = find_evidence(events, "incident.sh")
    met_call, _ = find_evidence(events, "dump-metrics")
    packets = [p for p in store["handoff_packets"] if p["kind"] in ("Restart", "Switch")]
    items = store["handoff_packet_items"]

    def form_of(packet_id: int, ref: int | None) -> str:
        if ref is None:
            return "none"
        forms = [i for i in items if i["packet_id"] == packet_id and i["ref_id"] == ref and i["zone"] == "Competing"]
        if not forms:
            return "absent"
        return forms[0]["form"] or ("dropped:" + str(forms[0]["reason"]))

    pk = packets[-1] if packets else None
    grades = res["grades"]
    zero_grade = {"header_ok": False, "timeout_ok": False, "retries_ok": False, "slow_ok": False, "visible_tests_ok": False}
    f2_runs = {r["id"] for r in runs if r["input_id"] == input_of.get("f2")}
    f2_text = "".join(e["body"]["Text"]["text"] for e in events if e["run_id"] in f2_runs and "Text" in e["body"])
    g1, g3 = grades.get("f1", zero_grade), grades.get("f3", zero_grade)
    f1 = g1["header_ok"] and g1["timeout_ok"] and g1["retries_ok"] and g1["visible_tests_ok"]
    f2 = facts["ticket"] in f2_text
    f3 = g3["slow_ok"] and g3["header_ok"] and g3["timeout_ok"] and g3["visible_tests_ok"]
    judgment_ok = [j for j in compact if j["outcome"] == "Ok"]
    compact_selected = bool(pk) and any(i["packet_id"] == pk["id"] and i["selector"] == "compact" and i["form"] for i in items)
    row.update(
        usable=True,
        f1=int(f1), f2=int(f2), f3=int(f3), composite=int(f1 and f2 and f3),
        header_ok=int(g3["header_ok"]), timeout_ok=int(g1["timeout_ok"]),
        input_tokens=usage["input"], cache_read_tokens=usage["cache_read"], cache_write_tokens=usage["cache_write"],
        output_tokens=usage["output"], router_tokens=router_tokens, router_compact_tokens=router_compact_tokens,
        provider_cost=round(cost(by_provider), 3),
        provider_cost_out4=round(cost(by_provider, 4.0), 3),
        provider_cost_out8=round(cost(by_provider, 8.0), 3),
        total_cost=round(cost(by_provider) + router_tokens, 3),
        window_seconds=round((last_end - boundary_end) / 1000, 3),
        tool_calls=len(tool_calls), metrics_reruns=reruns_metrics, incident_reruns=reruns_incident,
        restarts=len(packets), packet_tokens=pk["estimated_tokens"] if pk else None, packet_state=pk["state"] if pk else None,
        incident_form=form_of(pk["id"], inc_call) if pk else "no_packet",
        metrics_form=form_of(pk["id"], met_call) if pk else "no_packet",
        compact_judgments=len(compact), compact_ok=len(judgment_ok), jev_applied=int(compact_selected),
        key_hits_in_stdout=0,
    )
    return row


def load(run: str) -> list[dict]:
    out = []
    for path in sorted(RAW.glob(f"{run}-*.json.gz")):
        with gzip.open(path, "rt", encoding="utf-8") as f:
            out.append(json.load(f))
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--run", default="formal")
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    rows = [process_trial(r) for r in load(args.run)]
    rows.sort(key=lambda r: (r["provider"], r["seed"], plan.ARMS.index(r["arm"])))
    text = json.dumps(rows, ensure_ascii=False, sort_keys=True, indent=1) + "\n"
    target = OUT / f"{args.run}-trials.json"
    if args.check:
        same = target.exists() and target.read_text() == text
        print("process byte-identical" if same else "process MISMATCH")
        return 0 if same else 1
    OUT.mkdir(parents=True, exist_ok=True)
    target.write_text(text)
    print(f"{len(rows)} trials -> {target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
