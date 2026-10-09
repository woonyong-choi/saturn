"""data/raw/*.jsonl의 마지막 시도를 시행 표로, 2026-10-01 이전 답을 비교 표로 만든다.

사용: python3 scripts/02-process.py [--check]
--check는 파일을 쓰지 않고 이미 있는 표와 같은 바이트인지 확인한다.
"""
import csv
import io
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import plan  # noqa: E402

ROOT = plan.ROOT
RAW = ROOT / "data" / "raw"
PROCESSED = ROOT / "data" / "processed"
LEGACY = ROOT.parent / "indirect-constraint-accuracy" / "data" / "raw"
CHOICE_N = len(plan.CHOICE_OPTIONS)


def band(role: str, answer: float | None) -> str:
    if answer is None:
        return ""
    if role == "is_constraint":
        return "auto" if answer >= 0.8 else "ask" if answer >= 0.7 else "none"
    return "replace" if answer >= 0.8 else "possible" if answer >= 0.5 else "none"


def parse(row: dict) -> dict:
    """응답 원문에서 답을 읽는다. 형식이 맞지 않으면 invalid, 응답이 없으면 failed."""
    out = {"status": "failed", "answer": "", "top_option": "", "confidence": "", "api_confidence": "", "probabilities": ""}
    reply = row["response"]
    if row["http_status"] != 200 or not isinstance(reply, dict):
        return out
    out["status"] = "invalid"
    answer = (reply.get("answers") or {}).get(row["role"])
    if not isinstance(answer, dict):
        return out
    if row["role"] == "relation_choice":
        probs = answer.get("probabilities")
        wanted = set(row["request"]["questions"]["relation_choice"]["criteria"])
        if not isinstance(probs, dict) or set(probs) != wanted:
            return out
        values = list(probs.values())
        if not all(type(v) in (int, float) and 0 <= v <= 1 for v in values) or abs(sum(values) - 1) >= 0.01:
            return out
        top = max(sorted(probs), key=lambda k: probs[k])
        out.update(status="ok", top_option=top, confidence=(CHOICE_N * probs[top] - 1) / (CHOICE_N - 1),
                   api_confidence=answer.get("confidence", ""), probabilities=json.dumps(probs, sort_keys=True))
        return out
    value = answer.get("noul")
    if type(value) not in (int, float) or not 0.0 <= value <= 1.0:
        return out
    out.update(status="ok", answer=float(value))
    return out


def trial_rows() -> list[dict]:
    attempts: dict[str, int] = {}
    last: dict[str, dict] = {}
    order: list[str] = []
    for phase in ("dev", "confirm"):
        path = RAW / f"{phase}.jsonl"
        if not path.exists():
            continue
        for line in path.open(encoding="utf-8"):
            if not line.strip():
                continue
            row = json.loads(line)
            tid = row["trial_id"]
            attempts[tid] = attempts.get(tid, 0) + 1
            if tid not in last:
                order.append(tid)
            if row["final"]:
                last[tid] = row
            else:
                last.setdefault(tid, row)
    rows = []
    for tid in order:
        row = last[tid]
        parsed = parse(row)
        usage = (row["response"] or {}).get("usage") if isinstance(row["response"], dict) else None
        answer = parsed["answer"]
        rows.append({
            "trial_id": tid, "phase": row["phase"], "group": row["group"], "role": row["role"], "item_id": row["item_id"],
            "variant": row["variant"], "rep": row["rep"], "attempts": attempts[tid], "http_status": row["http_status"],
            "status": parsed["status"], "answer": answer, "band": band(row["role"], answer if answer != "" else None),
            "top_option": parsed["top_option"], "confidence": parsed["confidence"], "api_confidence": parsed["api_confidence"],
            "option_order": "|".join(row["option_order"]) if row["option_order"] else "",
            "model": (row["response"] or {}).get("model", "") if isinstance(row["response"], dict) else "",
            "input_tokens": usage.get("input_tokens", "") if isinstance(usage, dict) else "",
            "output_tokens": usage.get("output_tokens", "") if isinstance(usage, dict) else "",
            "latency_ms": row["latency_ms"], "request_sha256": row["request_sha256"],
        })
    return rows


def legacy_rows() -> list[dict]:
    """2026-10-01 #152 수집의 기준 조건 답. 확인 항목만 남긴다."""
    inputs, pairs = plan.confirm_items()
    wanted = {i["id"] for i in inputs} | {p["id"] for p in pairs}
    rows = []
    for path in sorted(LEGACY.glob("*.jsonl")):
        for line in path.open(encoding="utf-8"):
            r = json.loads(line)
            if r["condition"] in ("baseline", "replaces") and r["item_id"] in wanted:
                rows.append({"item_id": r["item_id"], "role": r["question_id"], "status": r["status"], "answer": "" if r["answer"] is None else r["answer"],
                             "model": r["model"] or "", "ts_utc": r["ts_utc"]})
    return sorted(rows, key=lambda r: (r["role"], r["item_id"]))


def to_csv(rows: list[dict]) -> str:
    if not rows:
        return ""
    buffer = io.StringIO(newline="")
    writer = csv.DictWriter(buffer, fieldnames=list(rows[0]), lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows)
    return buffer.getvalue()


def outputs() -> dict[str, str]:
    return {"trials.csv": to_csv(trial_rows()), "legacy_152.csv": to_csv(legacy_rows())}


def main() -> int:
    files = outputs()
    if "--check" in sys.argv:
        bad = [name for name, text in files.items() if (PROCESSED / name).read_text(encoding="utf-8") != text]
        if bad:
            print(f"처리 결과가 저장된 표와 다르다: {', '.join(bad)}", file=sys.stderr)
            return 1
        print("처리 결과 바이트 일치")
        return 0
    PROCESSED.mkdir(parents=True, exist_ok=True)
    for name, text in files.items():
        (PROCESSED / name).write_text(text, encoding="utf-8", newline="")
    print(f"처리 끝: {', '.join(files)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
