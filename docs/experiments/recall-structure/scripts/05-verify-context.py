"""실험 세션의 자동 첨부 맥락을 대조하고 변동을 제외 없이 기록한다."""

from collections import Counter
import hashlib
import json
from pathlib import Path

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/recall-structure"
RUN = PRIVATE / "formal"


def load(path):
    return json.loads(path.read_text())


def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()


def fingerprint(path):
    events = [json.loads(line) for line in path.read_text().splitlines()]
    meta = next(e["payload"] for e in events if e["type"] == "session_meta")
    messages = [
        e["payload"]["content"]
        for e in events
        if e["type"] == "response_item" and e["payload"].get("role") == "developer"
    ]
    normalized = json.dumps(messages, sort_keys=True).replace(meta["cwd"], "<workdir>")
    return dict(
        developer_sha256=digest(normalized),
        base_sha256=digest(json.dumps(meta["base_instructions"], sort_keys=True)),
        developer_messages=len(messages),
        has_skill_catalog="<skills_instructions>" in normalized,
    )


def verify():
    paths = list((Path.home() / ".codex/sessions").rglob("rollout-*.jsonl"))
    checks = []
    for folder in sorted((RUN / "raw").glob("codex-*")):
        if not (folder / "result.json").exists():
            continue
        result = load(folder / "result.json")
        if result["status"] != "ok":
            continue
        snapshot = folder / "context-fingerprint.json"
        if snapshot.exists():
            value = load(snapshot)
        else:
            matches = [
                p for p in paths if p.name.endswith(result["session_id"] + ".jsonl")
            ]
            assert len(matches) == 1, folder.name
            value = fingerprint(matches[0])
            snapshot.write_text(json.dumps(value, indent=2) + "\n")
        checks.append(dict(run=folder.name, **value))
    counts = Counter((c["developer_sha256"], c["base_sha256"]) for c in checks)
    protocol = load(PRIVATE / "smoke/verification.json")
    expected = {
        (c["developer_sha256"], c["base_sha256"])
        for c in protocol["checks"]
        if c["provider"] == "codex"
    }
    value = dict(
        verified_runs=len(checks),
        planned_runs=len(load(RUN / "calls-plan.json")),
        complete=len(checks) == len(load(RUN / "calls-plan.json")),
        stable_context=len(counts) == 1,
        matches_protocol=set(counts) == expected,
        groups=[
            dict(developer_sha256=k[0], base_sha256=k[1], runs=v)
            for k, v in sorted(counts.items())
        ],
        checks=checks,
        excluded_runs=0,
    )
    (PUBLIC / "results/context-verification.json").write_text(
        json.dumps(value, indent=2) + "\n"
    )
    print(json.dumps({k: v for k, v in value.items() if k not in ["checks", "groups"]}))


if __name__ == "__main__":
    verify()
