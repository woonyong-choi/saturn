"""예산, 봉인, 요청 재현, 공개 데이터 경계와 분석 분모를 검사한다."""

from __future__ import annotations

import ast
import datetime as dt
import hashlib
import importlib
import json
import subprocess

from common import LIMITS, PRIVATE, PUBLIC, ROOT, SOURCE, legacy, read_json, read_rows
from metrics import aggregate, decision, paired, ratio, tail


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def contract_checks() -> None:
    r = dict(
        item_id="a",
        cluster="a",
        valid=True,
        form="pair",
        intent="release",
        target="c1",
        request_p=0.9,
        constraint_p=0.1,
        none_p=None,
        candidates={"c1": 0.9},
        request_bytes=100,
    )
    require(decision(r, 0.8)["actions"] == ["c1"], "full release not selected")
    partial = r | dict(intent="partial")
    require(
        aggregate([partial], 0.8)["limited_release"]["k"] == 1,
        "partial release safety denominator lost",
    )
    blocked = r | dict(constraint_p=0.8)
    require(not decision(blocked, 0.8)["actions"], "constraint gate not applied")
    invalid = r | dict(valid=False, intent="none")
    require(not decision(invalid, 0.8)["correct"], "invalid answer counted correct")
    choice = r | dict(
        form="choice",
        none_p=0.95,
        candidates={"c1": 0.05},
        request_p=0.05,
        intent="none",
    )
    require(not decision(choice, 0.8)["actions"], "none choice released a rule")
    require(ratio(0, 0)["value"] is None, "empty denominator not null")
    require(
        abs(tail(0, 160, 0.02, upper=False) - 0.98**160) < 1e-12,
        "binomial probability differs",
    )
    require(
        paired([r], [invalid], 0.8)["difference"] == -1,
        "paired comparison direction wrong",
    )
    collect = importlib.import_module("02-collect")
    q = {"pick": {"type": "choice", "criteria": {"c1": None, "none": None}}}
    require(
        collect.parse_answers(
            {"answers": {"pick": {"probabilities": {"c1": 9, "none": 0.1}}}}, q
        )["pick"]
        is None,
        "invalid probability accepted",
    )


# cost: io local sources, receipts and git subprocesses; basis: estimate
def main() -> None:
    contract_checks()
    for path in (PUBLIC / "scripts").glob("*.py"):
        ast.parse(path.read_text())
    calls = read_rows(PRIVATE / "calls.jsonl")
    require(
        len({c["trial_id"] for c in calls}) == len(calls), "duplicate call reservation"
    )
    for kind, limit in LIMITS.items():
        require(sum(c["kind"] == kind for c in calls) <= limit, "call budget exceeded")
    run = read_json(PRIVATE / "run.json")
    seal_time = int(
        subprocess.check_output(
            ["git", "show", "-s", "--format=%ct", run["design_commit"]],
            cwd=ROOT,
            text=True,
        ).strip()
    )
    require(
        all(
            dt.datetime.fromisoformat(c["ts_utc"]).timestamp() >= seal_time
            for c in calls
        ),
        "call before design seal",
    )
    sealed = subprocess.check_output(
        [
            "git",
            "show",
            run["design_commit"]
            + ":docs/experiments/constraint-cancel-request/design.md",
        ],
        cwd=ROOT,
    )
    require(sealed == (PUBLIC / "design.md").read_bytes(), "design changed after seal")
    require(
        subprocess.run(
            ["git", "merge-base", "--is-ancestor", run["design_commit"], "HEAD"],
            cwd=ROOT,
            check=False,
        ).returncode
        == 0,
        "seal not ancestor",
    )
    for name, digest in read_json(PRIVATE / "plan.json")["source_hashes"].items():
        require(
            hashlib.sha256((SOURCE / name).read_bytes()).hexdigest() == digest,
            "source changed",
        )
    for line in (PUBLIC / "data/SHA256SUMS").read_text().splitlines():
        digest, name = line.split("  ", 1)
        require(
            hashlib.sha256((PRIVATE / name).read_bytes()).hexdigest() == digest,
            "receipt checksum mismatch",
        )
    items = read_json(PRIVATE / "items.json")
    rows = read_rows(PRIVATE / "processed.jsonl")
    require(len(rows) == len(items) * 12, "measurement coverage mismatch")
    raw = read_rows(PRIVATE / "jev.jsonl")
    require(len({r["trial_id"] for r in raw}) == len(raw), "duplicate response receipt")
    request_hashes = read_json(PRIVATE / "request-hashes.json")
    for r in raw:
        base_id = r["trial_id"].rsplit("-r", 1)[0]
        digest = hashlib.sha256(
            json.dumps(r["request"], ensure_ascii=False).encode()
        ).hexdigest()
        require(request_hashes.get(base_id) == digest, "request hash mismatch")
        require(
            r["request_bytes"]
            == len(json.dumps(r["request"], ensure_ascii=False).encode()),
            "reported byte count mismatch",
        )
    reserved = {c["trial_id"] for c in calls if c["kind"] == "jev"}
    require(all(r["trial_id"] in reserved for r in raw), "unreserved request")
    require(
        all(r["request_bytes"] <= 100000 for r in raw), "request byte limit exceeded"
    )
    public = read_json(PUBLIC / "data/generated.json")
    require(
        {r["id"] for r in public}
        == {i["id"] for i in items if i["source"] == "synthetic"},
        "public generated coverage mismatch",
    )
    require(
        all(
            set(r)
            <= {
                "id",
                "category",
                "n",
                "text",
                "intent",
                "target",
                "rule_ids",
                "text_redacted",
            }
            for r in public
        ),
        "private fields in public data",
    )
    for i in items:
        require(
            i["target"] == "none"
            or i["target"] in {f"c{k + 1}" for k in range(i["n"])},
            "invalid target",
        )
    for path in (SOURCE / "conversations").glob("*.json"):
        for turn in read_json(path)["turns"]:
            text = legacy.mask_text(turn["text"])
            if len(text) >= 8:
                require(
                    not any(text in r["text"] for r in public),
                    "original utterance published",
                )
    for i in items:
        if i["source"] == "real":
            require(
                not any(i["text"] in r["text"] for r in public),
                "real utterance published",
            )
    summary = read_json(PUBLIC / "results/summary.json")
    require(
        summary["calls"]
        == {kind: sum(c["kind"] == kind for c in calls) for kind in LIMITS},
        "summary call count mismatch",
    )
    print(
        json.dumps(
            dict(
                verified=True,
                items=len(items),
                observations=len(rows),
                calls=summary["calls"],
            )
        )
    )


if __name__ == "__main__":
    main()
