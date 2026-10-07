"""합성 세션 회수와 자동 첨부 맥락의 반복 일치를 확인한다."""

import hashlib
import importlib.util
import json
from pathlib import Path

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
SMOKE = ROOT / ".local/experiments/recall-structure/smoke"


def load(path):
    return json.loads(path.read_text())


def verify():
    spec = importlib.util.spec_from_file_location(
        "score", PUBLIC.parent / "real-context-replay/scripts/03-analyze.py"
    )
    score = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(score)
    sources = list((Path.home() / ".codex/sessions").rglob("rollout-*.jsonl"))
    checks = []
    fingerprints = []
    for provider in ["claude", "codex"]:
        for repeat in [1, 2, 3]:
            folder = SMOKE / "raw" / f"{provider}-protocol-r{repeat}-control"
            result = load(folder / "result.json")
            actual_tool_calls = result.get("tool_calls", 0)
            if provider == "codex":
                raw = [
                    json.loads(line)
                    for name in ["ready", "answer"]
                    for line in (folder / f"{name}.jsonl").read_text().splitlines()
                ]
                items = [e["item"] for e in raw if e.get("type") == "item.completed"]
                errors = [i for i in items if i.get("type") == "error"]
                assert all(
                    i.get("message", "").startswith(
                        "Under-development features enabled: skip_host_skill_discovery."
                    )
                    for i in errors
                )
                actual_tool_calls = sum(
                    i.get("type") not in ["agent_message", "reasoning", "error"]
                    for i in items
                )
            valid = (
                result["status"] == "ok"
                and result.get("ready") == "Ready"
                and result.get("same_session") is True
            )
            valid = (
                valid
                and actual_tool_calls == 0
                and score.parsed(result.get("text", "")) == {"version": "3.7.2"}
            )
            valid = valid and len(result.get("usages", [])) == 2
            checked = dict(
                recorded_tool_calls=result.get("tool_calls", 0),
                actual_tool_calls=actual_tool_calls,
                provider=provider,
                repeat=repeat,
                valid=valid,
                input_tokens=score.tokens(result),
                output_tokens=sum(u["output_tokens"] for u in result["usages"]),
            )
            if provider == "codex":
                matches = [
                    p
                    for p in sources
                    if p.name.endswith(result["session_id"] + ".jsonl")
                ]
                assert len(matches) == 1
                events = [
                    json.loads(line) for line in matches[0].read_text().splitlines()
                ]
                meta = next(e["payload"] for e in events if e["type"] == "session_meta")
                messages = [
                    e["payload"]["content"]
                    for e in events
                    if e["type"] == "response_item"
                    and e["payload"].get("role") == "developer"
                ]
                normalized = json.dumps(messages, sort_keys=True).replace(
                    meta["cwd"], "<workdir>"
                )
                checked["developer_sha256"] = hashlib.sha256(
                    normalized.encode()
                ).hexdigest()
                checked["developer_messages"] = len(messages)
                checked["has_skill_catalog"] = "<skills_instructions>" in normalized
                checked["base_sha256"] = hashlib.sha256(
                    json.dumps(meta["base_instructions"], sort_keys=True).encode()
                ).hexdigest()
                fingerprints.append(
                    (checked["developer_sha256"], checked["base_sha256"])
                )
            checks.append(checked)
    result = dict(
        checks=checks,
        stable_codex_context=len(set(fingerprints)) == 1,
        passed=all(c["valid"] for c in checks) and len(set(fingerprints)) == 1,
    )
    (SMOKE / "verification.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    if not result["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    verify()
