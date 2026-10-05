"""온라인 단계 수집·가공·검증. collect | process | verify.

수집은 원응답(알림 전체)과 기록 저장소 행을 .runtime/ar/raw에 남기고, 가공은 그 원자료에서만 trial을 다시 만든다.
"""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
from statistics import median

import arms
import contract
import online
from arm_runtime import POLICY_VERSION, SEED, analysis, read, write

RAW = online.RUN / "raw"
TRIALS = online.RUN / "trials.jsonl"
SUMMARY = online.PUBLIC / "results/online-summary.json"
SUMS = online.PUBLIC / "data/online-SHA256SUMS"
SOURCE = {"codex": "claude", "claude": "codex"}  # target -> source provider
EVIDENCE_CMD = re.compile(r"saturn\s+evidence")


def seal() -> None:
    """설계와 실행기를 커밋한 뒤에만 시작한다."""
    files = [online.PUBLIC / "design.md", *sorted((online.PUBLIC / "scripts").glob("*.py"))]
    for f in files:
        subprocess.run(["git", "diff", "--exit-code", "HEAD", "--", str(f)], cwd=online.ROOT, check=True, stdout=subprocess.DEVNULL)
    engine = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=online.BINARY.parents[2], text=True).strip()
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=online.ROOT, text=True).strip()
    (online.RUN).mkdir(parents=True, exist_ok=True)
    write(online.RUN / "seal.json", {"experiment_commit": commit, "engine_commit": engine,
                                    "files": {str(f.relative_to(online.ROOT)): hashlib.sha256(f.read_bytes()).hexdigest() for f in files}})


def collect() -> None:
    seal()
    RAW.mkdir(parents=True, exist_ok=True)
    c = online.case()
    used = {"claude": 0, "codex": 0, "router": 0}
    snaps = {}
    for target in ("codex", "claude"):
        source = SOURCE[target]
        snaps[target] = online.build_snapshot(source, c)["chat"]
        used[source] += 1
        used["router"] += 1
    # 조건 순서는 반복마다 시드로 섞는다
    import random
    plan = []
    for rep in range(online.REPS):
        for target in ("codex", "claude"):
            order = list(online.ARMS)
            random.Random(SEED + rep * 7 + len(target)).shuffle(order)
            plan += [(target, arm, rep) for arm in order]
    for target, arm, rep in plan:
        if used[target] >= online.LIMITS[target] or used["router"] >= online.LIMITS["router"]:
            print("call limit reached; stop")
            break
        record = online.run_trial(SOURCE[target], target, arm, rep, c, snaps[target])
        used[target] += 1 + len(record["db"]["packets"])
        used["router"] += len([j for j in record["db"]["judgments"] if j["started_at"] >= int(record["t0"] * 1000)])
        write(RAW / f"{record['name']}.json", record)
        print("collected", record["name"], record["status"], flush=True)
    write(online.RUN / "usage-count.json", used)


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def tool_blocks(events: list[dict]) -> dict[int, str]:
    """기록 번호 -> 블록 ID. 블록 파일을 읽은 도구 호출만 본다."""
    out = {}
    for e in events:
        m = re.search(r"rec-(b\d+)\.txt", e["body"])
        if e["body"].startswith('{"ToolCall"') and m:
            out[e["seq"]] = m.group(1)
    return out


def final_answer(notes: list[dict]):
    text = ""
    for n in notes:
        ev = n.get("params", {}).get("event", {}) if n.get("method") == "TaskEvent" else {}
        if "PacketReply" in ev:
            text = ""
        elif "Text" in ev:
            text += ev["Text"]["text"]
    for start in [m.start() for m in re.finditer(r"\{", text)]:
        for end in range(len(text), start, -1):
            if text[end - 1] == "}":
                try:
                    return json.loads(text[start:end])
                except ValueError:
                    continue
    return None


def usage_entries(record: dict) -> list[dict]:
    db, name, since = record["db"], record["name"], int(record["t0"] * 1000)
    out = []
    rows = [u for u in db["usage"] if u["at"] >= since]
    cumulative = {}
    for u in rows:
        if u["scope"] == "MainTurn":
            out.append(_entry(f"{name}-u{u['id']}", "parent", u))
        else:  # 스레드 누적값은 session마다 마지막 행이 그 session의 합이다
            cumulative[u["session_id"]] = u
    for sid, u in sorted(cumulative.items()):
        out.append(_entry(f"{name}-s{sid}", "parent", u))
    for j in db["judgments"]:
        if j["started_at"] >= since:
            out.append(dict(call_id=f"{name}-j{j['id']}", role="jev", parent_call_id=None, includes_children=False,
                            input_tokens=j["input_tokens"], cache_creation_input_tokens=None,
                            cache_read_input_tokens=None, output_tokens=j["output_tokens"]))
    return out


def _entry(call_id: str, role: str, u: dict) -> dict:
    return dict(call_id=call_id, role=role, parent_call_id=None, includes_children=False,
                input_tokens=u["input_tokens"], cache_creation_input_tokens=u["cache_write_tokens"],
                cache_read_input_tokens=u["cache_read_tokens"], output_tokens=u["output_tokens"])


def build_trial(record: dict, c: dict, raw_sha: str, raw_name: str) -> dict:
    db = record["db"]
    packets = db["packets"]
    first = packets[0] if packets else None
    blocks = tool_blocks(db["events"])
    forms: dict[str, str] = {}
    selectors = set()
    if first:
        for i in db["items"]:
            if i["packet_id"] == first["id"] and i["zone"] == "Competing":
                selectors.add(i["selector"])
                if i["ref_id"] in blocks:
                    forms[blocks[i["ref_id"]]] = i["form"] or ("omitted:" + str(i["reason"]))
    support = c["support_ids"]
    commands = [e["body"] for e in db["events"] if e["body"].startswith('{"ToolCall"') and EVIDENCE_CMD.search(e["body"]) and "--help" not in e["body"]]
    help_commands = sum(1 for e in db["events"] if e["body"].startswith('{"ToolCall"') and EVIDENCE_CMD.search(e["body"]) and "--help" in e["body"])
    reads = [l for l in db["lookups"] if l["kind"] == "Read"]
    read_blocks = sorted({blocks[l["record_id"]] for l in reads if l["outcome"] == "Ok" and l["record_id"] in blocks})
    answer = final_answer(record["notes"])
    state = first["state"] if first else None
    status = {"ok": "ok", "timeout": "incomplete"}.get(record["status"], "failed")
    if record["status"] == "ok" and any(v == "Failed" for v in (record.get("tasks") or {}).values()):
        status = "failed"
    if state in ("Unknown", "Prepared"):
        status = "delivery_unknown"
    usage = usage_entries(record)
    reported = sorted({u["model"] for u in db["usage"] if u["at"] >= int(record["t0"] * 1000) and u["model"]})
    return dict(
        task_id=c["task_id"], family_id=c["family_id"], split=c["split"], fixture_hash=c["fixture_hash"],
        arm=record["arm"], seed=SEED, provider=record["target"], model=online.PROVIDERS[record["target"]],
        model_version=reported[0] if reported else None, policy_version=POLICY_VERSION,
        input_id=first["input_id"] if first else None, run_id=first["run_id"] if first else None,
        session_id=first["session_id"] if first else None,
        decision_id=record["name"], started_at=record["started_at"], ended_at=record["ended_at"],
        latency_s=record["latency_s"], raw_files=[dict(name=raw_name, sha256=raw_sha)], raw_sha256=raw_sha,
        selection=dict(
            ids=sorted(b for b, f in forms.items() if f == "Full"), forms=dict(sorted(forms.items())),
            selector=sorted(s for s in selectors if s), selector_requested=record["arm"],
            support_ids=support, support_forms={b: forms.get(b) for b in support},
            candidate_hash=sha(json.dumps(arms.block_ids(c)).encode()),
        ),
        answer=answer, check=dict(grader="exact-v1", success=bool(status == "ok" and arms.grade(c, answer))),
        status=status, usage=usage,
        online=dict(
            overrides=record["overrides"], rep=record["rep"], source=record["source"],
            packet_hashes=[p["body_hash"] for p in packets], packet_states=[p["state"] for p in packets],
            packet_kinds=[p["kind"] for p in packets], packet_tokens=[p["estimated_tokens"] for p in packets],
            lookup_commands=len(commands), help_commands=help_commands, lookup_rows=len(db["lookups"]),
            lookup_outcomes={o: sum(l["outcome"] == o for l in db["lookups"]) for o in sorted({l["outcome"] for l in db["lookups"]})},
            support_read=read_blocks, denied=[d["summary"] for d in record["decisions"] if not d["allowed"]],
            judgments=[dict(id=j["id"], method=j["method"], outcome=j["outcome"], fallbacks=j["fallbacks"]) for j in db["judgments"] if j["started_at"] >= int(record["t0"] * 1000)],
        ),
    )


def load() -> list[tuple[str, bytes]]:
    return [(p.name, p.read_bytes()) for p in sorted(RAW.glob("*.json"))]


def process() -> None:
    c = online.case()
    rows = []
    for name, data in load():
        rows.append(build_trial(json.loads(data), c, sha(data), f"raw/{name}"))
    TRIALS.write_text("".join(json.dumps(r, ensure_ascii=False, sort_keys=True) + "\n" for r in rows))
    SUMS.parent.mkdir(exist_ok=True)
    SUMS.write_text("".join(f"{sha(d)}  raw/{n}\n" for n, d in load()))
    summarize(rows)
    print("processed", len(rows), "trials")


def summarize(rows: list[dict]) -> None:
    out: dict = {"evidence": "online engine path; one task, measurement check only, not a selector effect claim",
                 "task": online.FAMILY_TASK, "trials": len(rows), "cells": {}}
    for provider in ("claude", "codex"):
        for arm in online.ARMS:
            items = [r for r in rows if r["provider"] == provider and r["arm"] == arm]
            if not items:
                continue
            merged = [contract.merge_usage(r["usage"]) for r in items]
            lat = [r["latency_s"] for r in items]
            out["cells"][f"{provider}/{arm}"] = {
                "trials": len(items),
                "success": analysis.rate([r["check"]["success"] for r in items]),
                "statuses": {s: sum(r["status"] == s for r in items) for s in contract.STATUSES},
                "support_forms": {f: sum(sum(v == f for v in r["selection"]["support_forms"].values()) for r in items)
                                  for f in sorted({str(v) for r in items for v in r["selection"]["support_forms"].values()})},
                "selector_values": sorted({s for r in items for s in r["selection"]["selector"]}),
                "lookup_commands": [r["online"]["lookup_commands"] for r in items],
                "lookup_rows": [r["online"]["lookup_rows"] for r in items],
                "support_blocks_read": [len(r["online"]["support_read"]) for r in items],
                "restart_packets": sum(k == "Restart" for r in items for k in r["online"]["packet_kinds"]),
                "usage_known_sum": {k: sum(m["known"][k] for m in merged) for k in contract.USAGE_FIELDS},
                "usage_unreported_calls": sum(m["unreported_calls"] for m in merged),
                "latency_median_s": median(lat), "latency_sum_s": round(sum(lat), 6),
            }
    SUMMARY.parent.mkdir(exist_ok=True)
    SUMMARY.write_text(json.dumps(out, ensure_ascii=False, indent=2, sort_keys=True) + "\n")


def verify() -> None:
    c = online.case()
    rows = [json.loads(s) for s in TRIALS.read_text().splitlines()]
    raws = dict(load())
    expected = {(t, a, r) for t in ("claude", "codex") for a in online.ARMS for r in range(online.REPS)}
    got = {(r["provider"], r["arm"], r["online"]["rep"]) for r in rows}
    if got - expected or len(rows) != len(got):
        raise contract.ContractError("unexpected or duplicate trials")
    for r in rows:
        contract.validate_trial(r, online=True)
        contract.validate_usage(r["usage"])
        contract.merge_usage(r["usage"])
        if not r["online"]["packet_hashes"] or any(h is None for h in r["online"]["packet_hashes"]):
            raise contract.ContractError("packet hash missing: " + r["decision_id"])
        name = r["raw_files"][0]["name"].split("/", 1)[1]
        if sha(raws[name]) != r["raw_files"][0]["sha256"]:
            raise contract.ContractError("raw hash mismatch: " + name)
        if build_trial(json.loads(raws[name]), c, sha(raws[name]), "raw/" + name) != r:
            raise contract.ContractError("trial differs from raw: " + name)
        if "expected" in json.dumps(json.loads(raws[name])["overrides"]):
            raise contract.ContractError("answer leaked into settings")
    contract.check_cross_trial_usage(rows)
    before = SUMMARY.read_bytes()
    summarize(rows)
    if SUMMARY.read_bytes() != before:
        raise contract.ContractError("analysis is not byte-identical")
    print("online verify ok", len(rows))


if __name__ == "__main__":
    {"collect": collect, "process": process, "verify": verify}[sys.argv[1]]()
