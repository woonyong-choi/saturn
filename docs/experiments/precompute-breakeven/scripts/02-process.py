"""Check raw rows and flatten the latest run into processed/turns.csv."""

import argparse
import csv
import glob
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "..", "data")
SESSION_FIELDS = {"run_id": str, "trial_id": str, "condition": str, "ts_utc": str, "session_id": str,
                  "provider": str, "turns": list}
TURN_FIELDS = ("tool_calls", "tool_io_chars", "context_tokens", "compacted")


def latest_raw():
    files = sorted(glob.glob(os.path.join(DATA, "raw", "agent-logs-*.jsonl")))
    if not files:
        sys.exit("no raw file: run ./run.sh collect")
    return files[-1]


def check_row(row, line_no):
    errors = []
    for key, kind in SESSION_FIELDS.items():
        if not isinstance(row.get(key), kind):
            errors.append(f"line {line_no}: {key} missing or not {kind.__name__}")
    if row.get("provider") not in ("claude", "codex"):
        errors.append(f"line {line_no}: provider not in claude, codex")
    extra = set(row) - set(SESSION_FIELDS)
    if extra:
        errors.append(f"line {line_no}: unexpected fields {sorted(extra)}")
    for i, t in enumerate(row.get("turns") or []):
        if set(t) != set(TURN_FIELDS):
            errors.append(f"line {line_no} turn {i}: fields differ")
            continue
        if not (isinstance(t["tool_calls"], int) and t["tool_calls"] >= 0):
            errors.append(f"line {line_no} turn {i}: tool_calls")
        if not (isinstance(t["tool_io_chars"], int) and t["tool_io_chars"] >= 0):
            errors.append(f"line {line_no} turn {i}: tool_io_chars")
        if t["context_tokens"] is not None and not (isinstance(t["context_tokens"], int) and t["context_tokens"] >= 0):
            errors.append(f"line {line_no} turn {i}: context_tokens")
        if not isinstance(t["compacted"], bool):
            errors.append(f"line {line_no} turn {i}: compacted")
    return errors


def read_rows(path):
    rows, errors, ids = [], [], set()
    with open(path, encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            row = json.loads(line)
            errors.extend(check_row(row, n))
            if row.get("session_id") in ids:
                errors.append(f"line {n}: duplicate session_id")
            ids.add(row.get("session_id"))
            rows.append(row)
    return rows, errors


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--verify", action="store_true")
    args = ap.parse_args()
    raw = latest_raw()
    rows, errors = read_rows(raw)
    if errors:
        print("\n".join(errors[:20]), file=sys.stderr)
        sys.exit(1)
    if args.verify:
        print(f"ok: {len(rows)} sessions")
        return
    out_dir = os.path.join(DATA, "processed")
    os.makedirs(out_dir, exist_ok=True)
    with open(os.path.join(out_dir, "turns.csv"), "w", encoding="utf-8", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(["run_id", "session_id", "provider", "turn", *TURN_FIELDS])
        for row in rows:
            for i, t in enumerate(row["turns"], 1):
                ctx = "" if t["context_tokens"] is None else t["context_tokens"]
                w.writerow([row["run_id"], row["session_id"], row["provider"], i, t["tool_calls"],
                            t["tool_io_chars"], ctx, int(t["compacted"])])


if __name__ == "__main__":
    main()
