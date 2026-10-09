"""선택한 실제 대화와 SQLite 기록을 비공개 재생 자료로 고정한다."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import sqlite3
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = Path(
    os.environ.get(
        "SATURN_REPLAY_HOME", ROOT / ".local/experiments/real-context-replay/corrected"
    )
)


def save(path: Path, value: Any) -> None:
    with path.open("x") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def masker() -> Any:
    path = (
        PUBLIC.parent / "constraint-registration-generalization/scripts/01-prepare.py"
    )
    spec = importlib.util.spec_from_file_location("masking", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("masking module unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.masked


def turns(path: Path, mask: Any) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    for number, line in enumerate(path.open(), 1):
        row = json.loads(line)
        message = row.get("payload", {})
        if row.get("type") != "response_item" or message.get("type") != "message":
            continue
        role = message.get("role")
        text = "\n".join(x.get("text", "") for x in message.get("content", []))
        if role == "user":
            if text.startswith(("# AGENTS", "<", "This session")):
                continue
            result.append(
                {
                    "input": mask(text),
                    "answer": "",
                    "line": number,
                    "end": None,
                    "at_ms": int(
                        datetime.fromisoformat(
                            row["timestamp"].replace("Z", "+00:00")
                        ).timestamp()
                        * 1000
                    ),
                }
            )
        elif role == "assistant" and message.get("channel") != "analysis" and result:
            result[-1]["answer"] += mask(text) + "\n"
            if (
                message.get("channel") == "final"
                or message.get("phase") == "final_answer"
            ):
                result[-1]["end"] = "Completed"
    return result


def conversation_cases(
    sources: list[dict[str, Any]], mask: Any
) -> list[dict[str, Any]]:
    cases = []
    for prefix, name, stops in [
        ("43cbd2a41c0d", "learning", [12, 30, 59]),
        ("9da0d62259a3", "audit", [8, 16, 26]),
    ]:
        source = next(row for row in sources if row["source_id"].startswith(prefix))
        path = Path(source["source_path"])
        selected = turns(path, mask)
        save(
            PRIVATE / f"{name}-source.json",
            {
                "locator": str(path),
                "original_sha256": sha(path),
                "source_id": source["source_id"],
                "turns": selected,
            },
        )
        for stop in stops:
            rows = [
                {
                    "seq": index,
                    "run": index,
                    "session": 1,
                    "task": index,
                    "input": turn["input"],
                    "end": turn["end"],
                    "at_ms": turn["at_ms"],
                    "event": {
                        "Text": {"agent": 1, "subagent": None, "text": turn["answer"]}
                    },
                }
                for index, turn in enumerate(selected[:stop], 1)
            ]
            cases.append(
                {
                    "id": f"{name}-{stop}",
                    "cluster": name,
                    "rows": rows,
                    "source_lines": [turn["line"] for turn in selected[:stop]],
                }
            )
    return cases


def database_cases(mask: Any) -> list[dict[str, Any]]:
    cases = []
    for name, relative in [
        ("notes", "manual-615/evidence.db"),
        ("relay", "final-622/saturn.db"),
        ("interruption", "final-failure/saturn.db"),
    ]:
        path = ROOT / ".local/e2e" / relative
        connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
        query = """SELECT e.seq, r.id, r.session_id, r.task_id, i.text, r.end_kind, e.at, e.body
                   FROM events e JOIN runs r ON r.id=e.run_id
                   LEFT JOIN inputs i ON i.id=r.input_id ORDER BY e.seq"""
        rows = [
            {
                "seq": seq,
                "run": run,
                "session": session,
                "task": task,
                "input": mask(text) if text else None,
                "end": end,
                "at_ms": at,
                "event": json.loads(mask(event)),
            }
            for seq, run, session, task, text, end, at, event in connection.execute(
                query
            )
        ]
        connection.close()
        cases.append(
            {
                "id": name,
                "cluster": name,
                "rows": rows,
                "locator": str(path),
                "original_sha256": sha(path),
            }
        )
    return cases


def full_text(case: dict[str, Any]) -> str:
    sections = []
    previous = None
    for row in case["rows"]:
        if row["run"] != previous:
            sections.append(
                f"User [run {row['run']}, end {row['end']}]: {row['input']}"
            )
            previous = row["run"]
        sections.append(json.dumps(row["event"], ensure_ascii=False))
    return "\n".join(sections)


def main() -> None:
    os.umask(0o077)
    PRIVATE.mkdir(parents=True, exist_ok=True)
    mask = masker()
    sources = json.loads(
        (
            ROOT
            / ".local/experiments/constraint-registration-generalization/sources.json"
        ).read_text()
    )
    cases = conversation_cases(sources, mask) + database_cases(mask)
    for case in cases:
        case["full"] = full_text(case)
    save(PRIVATE / "cases.json", cases)
    paths = [PUBLIC / "design.md", PUBLIC / "questions.json", PRIVATE / "cases.json"]
    paths.extend(sorted((PUBLIC / "scripts").glob("*.py")))
    paths.extend(
        [
            ROOT / "saturn-terminal/engine/src/handoff.rs",
            ROOT / "saturn-terminal/engine/src/handoff_replay.rs",
            ROOT / "saturn-terminal/core/src/sessions/packet.rs",
        ]
    )
    save(
        PRIVATE / "seal.json",
        {
            "at": datetime.now(timezone.utc).isoformat(),
            "files": {str(p.relative_to(ROOT)): sha(p) for p in paths},
        },
    )
    print(
        json.dumps(
            {
                "cases": len(cases),
                "clusters": len({c["cluster"] for c in cases}),
                "rows": sum(len(c["rows"]) for c in cases),
            }
        )
    )


if __name__ == "__main__":
    main()
