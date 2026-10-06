"""봉인된 원자료만으로 12개 대응 쌍의 판정과 비용을 다시 계산한다."""

from __future__ import annotations

import importlib.util
import json
import random
import statistics
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("selector_collect", BASE / "scripts/01-collect.py")
collect = importlib.util.module_from_spec(spec)
spec.loader.exec_module(collect)


def load(path: Path) -> dict | None:
    return json.loads(path.read_text()) if path.exists() else None


def first_packet(record: dict | None) -> tuple[dict | None, list[dict], dict | None]:
    if not record:
        return None, [], None
    packets = [p for p in record["db"]["packets"] if p["created_at"] >= record["started_at_ms"] and p["kind"] == "Switch"]
    if not packets:
        return None, [], None
    packet = packets[0]
    items = [i for i in record["db"]["items"] if i["packet_id"] == packet["id"]]
    capture = next((c for c in record["captures"] if c["packet_id"] == packet["id"]), None)
    return packet, items, capture


def cost(record: dict | None, prices: dict) -> float | None:
    if not record:
        return None
    started = record["started_at_ms"]
    usage = [u for u in record["db"]["usage"] if u["at"] >= started and u["scope"] == "MainTurn"]
    judges = [j for j in record["db"]["judgments"] if j["started_at"] >= started]
    if not usage:
        return None
    total = 0.0
    for u in usage:
        p = prices["claude"]
        fields = (("input_tokens", "input"), ("cache_write_tokens", "cache_write"), ("cache_read_tokens", "cache_read"), ("output_tokens", "output"))
        if any(u[f] is None or p[k] is None for f, k in fields):
            return None
        total += sum(u[f] * p[k] / 1e6 for f, k in fields)
    for j in judges:
        p = prices["jev"]
        if j["input_tokens"] is None or j["output_tokens"] is None:
            return None
        total += (j["input_tokens"] * p["input"] + j["output_tokens"] * p["output"]) / 1e6
    return round(total, 8)


def one(index: int, arm: str, manifest: dict) -> dict:
    record = load(collect.RUN / "raw" / f"t{index:02}{arm}.json")
    packet, items, capture = first_packet(record)
    expected = "compact" if arm == "J" else "rank"
    _, hidden, current_name = collect.source_files(index)
    ok = bool(record and record["status"] == "ok" and record["tasks"] and all(s == "Done" for s in record["tasks"].values())
              and record["answer"].strip() == str(hidden))
    return {
        "arm": arm, "collected": bool(record), "answer": record["answer"].strip() if record else None,
        "success": ok, "current_name": current_name,
        "duration_s": record["duration_s"] if record else None,
        "cost_usd": cost(record, manifest["prices"]),
        "packet_hash": packet["body_hash"] if packet else None,
        "packet_bytes": packet["body_bytes"] if packet else None,
        "packet_id": packet["id"] if packet else None,
        "selector_applied": bool(packet and packet["requested_selector"] == expected and packet["actual_selector"] == expected and packet["selection_fallback"] is None),
        "competing": [(i["ref_id"], i["form"], i["reason"]) for i in items if i["zone"] == "Competing"],
        "protected": [(i["zone"], i["ref_id"], i["body_hash"]) for i in items if i["zone"] in ("User", "Assistant")],
        "capture_ok": bool(packet and capture and collect.sha(capture["body"].encode()) == packet["body_hash"] == capture["body_hash"]
                           and len(capture["body"].encode()) == packet["body_bytes"] == capture["body_bytes"]),
        "judgments": len([j for j in record["db"]["judgments"] if j["started_at"] >= record["started_at_ms"]]) if record else None,
        "usage_rows": len([u for u in record["db"]["usage"] if u["at"] >= record["started_at_ms"]]) if record else None,
    }


def bootstrap(values: list[float]) -> list[float] | None:
    if not values:
        return None
    rng = random.Random(collect.SEED)
    sampled = sorted(statistics.mean(rng.choices(values, k=len(values))) for _ in range(10000))
    return [sampled[249], sampled[9749]]


def analyze() -> dict:
    manifest = collect.check_seal()
    pairs = []
    for index in range(12):
        source = load(collect.RUN / "raw" / f"s{index:02}.json")
        arms = {arm: one(index, arm, manifest) for arm in ("R", "J")}
        r, j = arms["R"], arms["J"]
        packet_contrast = bool(r["packet_hash"] and j["packet_hash"] and r["packet_hash"] != j["packet_hash"])
        competing_contrast = bool(r["competing"] and j["competing"] and r["competing"] != j["competing"])
        protected_match = bool(r["protected"] and j["protected"] and r["protected"] == j["protected"])
        pairs.append({"id": index, "source_valid": bool(source and source["valid"]), "arms": arms,
                      "packet_contrast": packet_contrast, "competing_contrast": competing_contrast,
                      "protected_match": protected_match})
    full = [p for p in pairs if p["arms"]["R"]["collected"] and p["arms"]["J"]["collected"]]
    cost_diff = [p["arms"]["J"]["cost_usd"] - p["arms"]["R"]["cost_usd"] for p in full if all(p["arms"][a]["cost_usd"] is not None for a in ("R", "J"))]
    duration_diff = [p["arms"]["J"]["duration_s"] - p["arms"]["R"]["duration_s"] for p in full if all(p["arms"][a]["duration_s"] is not None for a in ("R", "J"))]
    b = sum(p["arms"]["J"]["success"] and not p["arms"]["R"]["success"] for p in pairs)
    c = sum(p["arms"]["R"]["success"] and not p["arms"]["J"]["success"] for p in pairs)
    h1 = sum(p["packet_contrast"] and p["competing_contrast"] and p["protected_match"] for p in pairs) >= 6 and all(p["protected_match"] for p in full)
    h2 = h1 and b > c
    cost_ci, duration_ci = bootstrap(cost_diff), bootstrap(duration_diff)
    h3 = bool(h2 and len(cost_diff) == len(duration_diff) == 12 and cost_ci and duration_ci and cost_ci[1] < 0 and duration_ci[1] < 0)
    summary = {"planned_pairs": 12, "source_valid": sum(p["source_valid"] for p in pairs), "complete_pairs": len(full),
        "selector_applied": {a: sum(p["arms"][a]["selector_applied"] for p in pairs) for a in ("R", "J")},
        "success": {a: sum(p["arms"][a]["success"] for p in pairs) for a in ("R", "J")},
        "packet_contrast": sum(p["packet_contrast"] for p in pairs),
        "competing_contrast": sum(p["competing_contrast"] for p in pairs),
        "joint_contrast": sum(p["packet_contrast"] and p["competing_contrast"] and p["protected_match"] for p in pairs),
        "protected_mismatch": sum(not p["protected_match"] for p in full),
        "jev_only_correct": b, "rrf_only_correct": c,
        "cost_diff_mean_usd": statistics.mean(cost_diff) if cost_diff else None, "cost_diff_ci95_usd": cost_ci,
        "duration_diff_mean_s": statistics.mean(duration_diff) if duration_diff else None, "duration_diff_ci95_s": duration_ci,
        "h1": h1, "h2": h2, "h3": h3, "pairs": pairs}
    return summary


if __name__ == "__main__":
    summary = analyze()
    collect.save(collect.RUN / "summary.json", summary)
    print(json.dumps({k: v for k, v in summary.items() if k != "pairs"}, ensure_ascii=False, indent=2))
