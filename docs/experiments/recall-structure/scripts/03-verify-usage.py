"""원시 사용량과 세션 자체의 턴별 사용량을 독립적으로 대조한다."""

from __future__ import annotations

import json
from pathlib import Path
import re

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
RUN = ROOT / ".local/experiments/recall-structure/formal"


def load(path):
    return json.loads(path.read_text())


def events(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def verify():
    sessions = Path.home() / ".codex/sessions"
    paths = list(sessions.rglob("rollout-*.jsonl"))
    checked = []
    for folder in sorted((RUN / "raw").iterdir()):
        result_path = folder / "result.json"
        if not result_path.exists():
            continue
        result = load(result_path)
        if result["status"] != "ok":
            continue
        if result["provider"] == "codex":
            thread = result["session_id"]
            first_command = load(folder / "ready-command.json")
            second_command = load(folder / "answer-command.json")
            assert first_command[:2] == ["codex", "exec"]
            assert second_command[:3] == ["codex", "exec", "resume"]
            assert second_command[-2:] == [thread, "-"]
            assert "--ephemeral" not in first_command
            for command in [first_command, second_command]:
                assert command[command.index("-m") + 1] == "gpt-6-sol"
                assert 'sandbox_mode="read-only"' in command
            assert re.fullmatch(r"[0-9a-f-]{36}", thread)
            snapshots = folder / "token-events.json"
            if snapshots.exists():
                counts = load(snapshots)
            else:
                matched = [p for p in paths if p.name.endswith(f"{thread}.jsonl")]
                assert len(matched) == 1
                counts = [
                    e["payload"]["info"]
                    for e in events(matched[0])
                    if e.get("payload", {}).get("type") == "token_count"
                    and e["payload"].get("info")
                ]
                snapshots.write_text(json.dumps(counts, indent=2) + "\n")
            # 같은 누적값의 반복 알림은 한 번만 센다.
            unique = []
            for count in counts:
                if not unique or count != unique[-1]:
                    unique.append(count)
            assert len(unique) == 2, (folder.name, len(unique))
            for index, count in enumerate(unique):
                for key in ["input_tokens", "output_tokens", "cached_input_tokens"]:
                    assert (
                        count["last_token_usage"][key] == result["usages"][index][key]
                    )
                    assert (
                        count["total_token_usage"][key]
                        == result["cumulative_usages"][index][key]
                    )
                    assert result["usages"][index][key] >= 0
            checked.append(
                dict(
                    run=folder.name,
                    source="native total and last token usage",
                    input_tokens=sum(u["input_tokens"] for u in result["usages"]),
                )
            )
        else:
            raw = [
                e
                for name in ["ready", "answer"]
                for e in events(folder / f"{name}.jsonl")
                if e.get("type") == "result"
            ]
            assert len(raw) == 2
            native = raw[-1]["modelUsage"]
            native_input = sum(
                u["inputTokens"]
                + u["cacheReadInputTokens"]
                + u["cacheCreationInputTokens"]
                for u in native.values()
            )
            measured = sum(
                u.get("input_tokens", 0)
                + u.get("cache_read_input_tokens", 0)
                + u.get("cache_creation_input_tokens", 0)
                for u in result["usages"]
            )
            assert native_input == measured, (folder.name, native_input, measured)
            assert sum(u["outputTokens"] for u in native.values()) == sum(
                u["output_tokens"] for u in result["usages"]
            )
            checked.append(
                dict(
                    run=folder.name,
                    source="native cumulative modelUsage",
                    input_tokens=measured,
                )
            )
    verify_payloads()
    (PUBLIC / "results/usage-verification.json").write_text(
        json.dumps(
            dict(verified_runs=len(checked), runs=checked), ensure_ascii=False, indent=2
        )
        + "\n"
    )
    print(f"independently verified usage: {len(checked)}")


def verify_payloads():
    for job in load(RUN / "calls-plan.json"):
        if job["arm"] != "joint":
            continue
        packet = load(RUN / "baseline/packets" / f"{job['case']}-4000.json")["text"]
        original = load(RUN / "repaired/recall" / f"{job['case']}.json")["input"]
        tag = re.search(r"<(saturn-history(?:-x)*)>\n", job["question"])[1]
        archive, current = (
            job["question"]
            .split(f"<{tag}>\n", 1)[1]
            .split(f"\n</{tag}>\n\nCurrent user input:\n", 1)
        )
        assert archive.encode() == packet.encode()
        assert current.encode() == original.encode()


if __name__ == "__main__":
    verify()
