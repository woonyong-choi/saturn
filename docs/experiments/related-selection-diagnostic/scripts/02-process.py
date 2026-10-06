#!/usr/bin/env python3
"""비공개 실행 원자료를 조건별 수치 행으로 변환한다."""

from __future__ import annotations

import csv
import hashlib
import json
import sys
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
ROOT = BASE.parents[2]
FIELDS = ("case", "arm", "status", "answer", "expected", "correct", "first_packet_hash", "first_packet_bytes", "related_applied", "lookups", "provider_tokens", "router_tokens", "latency_s")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def rows(run_id: str) -> tuple[list[dict], list[tuple[str, str]]]:
    folder = ROOT / ".runtime" / "related-selection-diagnostic" / run_id
    if not (folder / "manifest.json").is_file():
        raise FileNotFoundError("run manifest missing")
    result = []
    hashes = []
    for source in sorted(folder.glob("*/source.json")):
        case = source.parent.name
        hashes.append((digest(source), f"{case}/source.json"))
        for arm in ("off", "rank", "jev"):
            trial = source.parent / f"{arm}.json"
            if not trial.exists():
                result.append(dict(case=case, arm=arm, status="missing", answer="", expected="", correct=0, first_packet_hash="", first_packet_bytes="", related_applied=0, lookups="", provider_tokens="", router_tokens="", latency_s=""))
                continue
            hashes.append((digest(trial), f"{case}/{arm}.json"))
            raw = json.loads(trial.read_text(encoding="utf-8"))
            db = raw["db"]
            packets = db["packets"]
            first = packets[0] if packets else {}
            judgments = db["judgments"]
            related = any("related@2.0" in item.get("question_sets", "") for item in judgments)
            provider_tokens = sum(sum(item.get(key, 0) or 0 for key in ("input_tokens", "cache_write_tokens", "cache_read_tokens", "output_tokens")) for item in db["usage"])
            router_tokens = sum((item.get("input_tokens") or 0) + (item.get("output_tokens") or 0) for item in judgments)
            result.append(dict(case=case, arm=arm, status=raw["status"], answer=raw["answer"], expected=raw["expected"], correct=int(raw["status"] == "ok" and raw["answer"] == str(raw["expected"])), first_packet_hash=first.get("body_hash", ""), first_packet_bytes=first.get("body_bytes", ""), related_applied=int(related), lookups=len(db["lookups"]), provider_tokens=provider_tokens, router_tokens=router_tokens, latency_s=round(raw["latency_s"], 3)))
    hashes.insert(0, (digest(folder / "manifest.json"), "manifest.json"))
    return result, hashes


def render_csv(data: list[dict]) -> str:
    import io

    out = io.StringIO(newline="")
    writer = csv.DictWriter(out, fieldnames=FIELDS)
    writer.writeheader()
    writer.writerows(data)
    return out.getvalue()


def main() -> None:
    run_id = sys.argv[1]
    data, hashes = rows(run_id)
    processed = BASE / "data" / "processed"
    processed.mkdir(parents=True, exist_ok=True)
    (processed / "trials.csv").write_text(render_csv(data), encoding="utf-8")
    (BASE / "data" / "SHA256SUMS").write_text("".join(f"{value}  {name}\n" for value, name in hashes), encoding="utf-8")
    print(f"processed {len(data)} trial rows")


if __name__ == "__main__":
    main()
