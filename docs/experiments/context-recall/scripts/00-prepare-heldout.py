"""고정한 Claude 원본을 다시 읽어 마스킹된 보류 자료를 재현한다."""

from __future__ import annotations

import argparse
from datetime import datetime
import hashlib
import importlib.util
import json
from pathlib import Path

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    expected = json.loads(
        (
            ROOT / ".local/experiments/context-recall/heldout-design-source.json"
        ).read_text()
    )
    path = Path(expected["locator"])
    if hashlib.sha256(path.read_bytes()).hexdigest() != expected["original_sha256"]:
        raise RuntimeError("original source changed")
    spec = importlib.util.spec_from_file_location(
        "source_adapter", PUBLIC.parent / "real-context-replay/scripts/01-prepare.py"
    )
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    mask = helper.masker()
    turns = []
    for number, line in enumerate(path.open(), 1):
        row = json.loads(line)
        message = row.get("message", {})
        content = message.get("content", "")
        text = (
            content
            if isinstance(content, str)
            else "\n".join(
                part.get("text", "") for part in content if part.get("type") == "text"
            )
        )
        if (
            row.get("type") == "user"
            and text
            and not text.startswith(("<", "This session"))
        ):
            turns.append(
                dict(
                    input=mask(text),
                    answer="",
                    line=number,
                    end=None,
                    at_ms=int(
                        datetime.fromisoformat(
                            row["timestamp"].replace("Z", "+00:00")
                        ).timestamp()
                        * 1000
                    ),
                )
            )
        elif row.get("type") == "assistant" and turns and text:
            turns[-1]["answer"] += mask(text) + "\n"
            if message.get("stop_reason") == "end_turn":
                turns[-1]["end"] = "Completed"
    source = dict(
        locator=str(path),
        original_sha256=expected["original_sha256"],
        source_id=expected["source_id"],
        turns=turns,
    )
    rows = [
        dict(
            seq=i,
            run=i,
            session=1,
            task=i,
            input=turn["input"],
            end=turn["end"],
            at_ms=turn["at_ms"],
            event={"Text": dict(agent=1, subagent=None, text=turn["answer"])},
        )
        for i, turn in enumerate(turns, 1)
    ]
    case = dict(
        id="design-chain",
        cluster="design-chain",
        rows=rows,
        source_lines=[turn["line"] for turn in turns],
        locator=source["locator"],
        original_sha256=source["original_sha256"],
        source_kinds=[
            "human_request",
            "injected_skill",
            "injected_skill",
            "human_request",
            "interrupt_marker",
        ],
    )
    case["full"] = helper.full_text(case)
    args.output.mkdir(parents=True, exist_ok=False)
    for name, value in [
        ("heldout-design-source.json", source),
        ("heldout-case.json", case),
    ]:
        (args.output / name).write_text(json.dumps(value, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
