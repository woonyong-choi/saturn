"""평가 뒤 단일 JSON 코드 블록의 포장만 제거한 탐색 자료를 만든다."""

from __future__ import annotations

import json
import re

from protocol import validate
from runtime import PRIVATE, read


def unwrap(row: dict, item: dict) -> dict:
    result = {**row, "exploratory_unwrapped": False}
    if row["status"] == "ok":
        return result
    path = PRIVATE / "claude" / row["trial_id"] / "receipt.json"
    if not path.exists():
        return result
    receipt = read(path)
    try:
        wrapper = json.loads(receipt["stdout"])
        if receipt["returncode"] != 0 or wrapper.get("is_error"):
            return result
        blocks = re.findall(
            r"```(?:json)?\s*\n(.*?)\n```", wrapper.get("result", ""), re.S
        )
        if len(blocks) != 1:
            return result
        prediction = json.loads(blocks[0])
        valid = (
            (
                isinstance(prediction, dict)
                and set(prediction) == {"continue"}
                and type(prediction["continue"]) is bool
            )
            if item["task"] == "continuation"
            else validate(prediction, item)
        )
        if valid:
            result.update(
                status="ok", prediction=prediction, exploratory_unwrapped=True
            )
    except (ValueError, KeyError, TypeError):
        return result
    return result
