"""비공개 응답을 라벨과 합쳐 분석용 관측 행을 만든다."""

from __future__ import annotations

import hashlib
import importlib
import json

from common import PRIVATE, PUBLIC, SOURCE, legacy, read_json, read_rows, write_json

collect = importlib.import_module("02-collect")


# cost: time O(n), heap O(n), io local source and receipt files; vars: n = observations; basis: estimate
def main() -> None:
    items = read_json(PRIVATE / "items.json")
    public = []
    originals = {
        legacy.mask_text(t["text"])
        for p in (SOURCE / "conversations").glob("*.json")
        for t in read_json(p)["turns"]
        if len(t["text"]) >= 8
    }
    for item in items:
        if item["source"] != "synthetic":
            continue
        row = {k: item[k] for k in ("id", "category", "n", "text", "intent", "target")}
        text = legacy.mask_text(row["text"])
        for original in sorted(originals, key=len, reverse=True):
            if original in text:
                text = text.replace(original, "[원문 가림]")
        row["text_redacted"] = text != row["text"]
        row["text"] = text
        row["rule_ids"] = [
            "r-" + hashlib.sha256(k.encode()).hexdigest()[:12] for k in item["rule_ids"]
        ]
        public.append(row)
    write_json(PUBLIC / "data/generated.json", public)
    raw = {r["trial_id"]: r for r in read_rows(PRIVATE / "jev.jsonl")}
    run = read_json(PRIVATE / "run.json")
    rows = []
    for i in items:
        cluster = (
            i["id"].rsplit("-u-", 1)[0]
            if i["source"] == "real"
            else i["rule_ids"][
                int(i["target"][1:]) - 1 if i["target"] != "none" else -1
            ]
        )
        for form in ("pair", "choice"):
            for record in (False, True):
                t = collect.trial(i, form, record)
                expected = {k: t[k] for k in ("model", "state", "questions")}
                for repeat in (1, 2, 3):
                    trial_id = t["trial_id"] + f"-r{repeat}"
                    r = raw.get(trial_id, {})
                    if r and r["request"] != expected:
                        raise RuntimeError("request differs from preregistered input")
                    parsed = collect.parse_answers(
                        r.get("raw_response", {}), t["questions"]
                    )
                    valid = (
                        bool(r)
                        and r.get("status") == "ok"
                        and all(p is not None for p in parsed.values())
                    )
                    candidates = {f"c{k + 1}": None for k in range(i["n"])}
                    request_p, constraint_p, none_p = None, None, None
                    if valid:
                        if form == "pair":
                            request_p = parsed["is_release"]
                            constraint_p = parsed["is_constraint"]
                            candidates = {
                                f"c{k + 1}": parsed[f"releases_{k + 1}"]
                                for k in range(i["n"])
                            }
                        else:
                            ps = parsed["release_target"]
                            none_p = ps["none"]
                            request_p = 1 - none_p
                            candidates = {k: ps[k] for k in candidates}
                    rows.append(
                        dict(
                            run_id=run["run_id"],
                            trial_id=trial_id,
                            condition=f"{form}-{int(record)}",
                            ts_utc=r.get("ts_utc"),
                            item_id=i["id"],
                            cluster=cluster,
                            form=form,
                            record=record,
                            repeat=repeat,
                            n=i["n"],
                            category=i["category"],
                            source=i["source"],
                            intent=i["intent"],
                            target=i["target"],
                            status=r.get("status", "incomplete"),
                            valid=valid,
                            request_p=request_p,
                            constraint_p=constraint_p,
                            none_p=none_p,
                            candidates=candidates,
                            request_bytes=r.get(
                                "request_bytes",
                                len(json.dumps(expected, ensure_ascii=False).encode()),
                            ),
                        )
                    )
    with (PRIVATE / "processed.jsonl").open("w") as stream:
        for row in rows:
            stream.write(json.dumps(row, ensure_ascii=False) + "\n")
    paths = sorted(
        p
        for p in PRIVATE.rglob("*")
        if p.is_file()
        and (
            p.name
            in (
                "items.json",
                "jev.jsonl",
                "calls.jsonl",
                "plan.json",
                "flow.json",
                "run.json",
                "excluded.json",
                "request-hashes.json",
            )
            or p.name == "receipt.json"
        )
    )
    lines = [
        hashlib.sha256(p.read_bytes()).hexdigest() + "  " + str(p.relative_to(PRIVATE))
        for p in paths
    ]
    (PUBLIC / "data/SHA256SUMS").write_text("\n".join(lines) + "\n")
    print(json.dumps(dict(processed=len(rows), observed=len(raw))))


if __name__ == "__main__":
    main()
