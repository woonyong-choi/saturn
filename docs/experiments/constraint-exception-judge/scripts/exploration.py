"""평가 뒤 단일 JSON 코드 블록의 포장만 제거한 탐색 자료를 만든다."""

from __future__ import annotations

import importlib
import json
import re
from collections import Counter
from decimal import Decimal

from protocol import MODEL, scope_candidates, validate
from runtime import PRIVATE, read


def tie_order_audit(rows: list[dict], items: dict) -> dict:
    collector = importlib.import_module("02-collect")
    counts = Counter()
    changed = []
    for row in rows:
        if row["condition"] not in ("J1", "J2"):
            continue
        answers = {}
        for stage in row["stages"]:
            receipt = read(PRIVATE / "jev" / (stage + ".json"))
            reply = json.loads(receipt["raw_response"])
            for name, question in receipt["request"]["questions"].items():
                if question["type"] != "choice":
                    continue
                values = reply["answers"][name]["probabilities"]
                keys = list(question["criteria"])
                if sum(v == max(values.values()) for v in values.values()) > 1:
                    counts["tied_questions"] += 1
                if max(values, key=values.get) != max(keys, key=values.get):
                    counts["different_choice_winners"] += 1
                if receipt["status"] == "ok":
                    answers[name] = {k: values[k] for k in keys}
        if not all(k in answers for k in ("release_target", "kind", "scope")):
            continue
        # 기존 호출 결과만 사용하며 아직 호출하지 않은 단계는 만들지 않는다.
        prediction = collector.decode(items[row["item_id"]], answers, row["request_p"])
        status = "ok" if validate(prediction, items[row["item_id"]]) else "invalid"
        if prediction != row["prediction"] or status != row["status"]:
            changed.append(row)
    return dict(
        **counts,
        changed_workflows=len(changed),
        changed_first_repeat=sum(r["repeat"] == 1 for r in changed),
        scope="J1 and J2; unused B1 auxiliary questions excluded",
    )


def decimal_sensitivity(row: dict, item: dict) -> dict:
    result = {**row, "exploratory_decimal": False}
    if row["condition"] != "J1" or row["status"] != "invalid":
        return result
    receipt = read(PRIVATE / "jev" / (row["trial_id"] + ".json"))
    if receipt["status"] != "invalid":
        return result
    reply = json.loads(receipt["raw_response"], parse_float=Decimal)
    if reply.get("model") != MODEL:
        return result
    expected = {
        "release_target": {"none", *(f"c{i + 1}" for i in range(len(item["rules"])))},
        "kind": {"permanent", "once", "scoped"},
        "scope": set(scope_candidates(item["text"])),
    }
    answers = reply.get("answers", {})
    if set(answers) != set(expected):
        return result
    probabilities = {}
    for name, choices in expected.items():
        values = answers[name].get("probabilities", {})
        if set(values) != choices or any(
            type(v) not in (int, Decimal)
            or (isinstance(v, Decimal) and not v.is_finite())
            for v in values.values()
        ):
            return result
        if any(not 0 <= v <= 1 for v in values.values()):
            return result
        if abs(sum(values.values()) - 1) > Decimal("0.01"):
            return result
        probabilities[name] = values
    if 1 - probabilities["release_target"]["none"] < Decimal("0.50"):
        prediction = dict(request="none", target="none", kind="none", scope_text=None)
    else:
        target = max(
            (k for k in probabilities["release_target"] if k != "none"),
            key=probabilities["release_target"].get,
        )
        kind = max(probabilities["kind"], key=probabilities["kind"].get)
        scope = scope_candidates(item["text"])[
            max(probabilities["scope"], key=probabilities["scope"].get)
        ]
        prediction = dict(
            request="release" if kind == "permanent" else "exception",
            target=target,
            kind=kind,
            scope_text=None if kind == "permanent" else scope,
        )
    if validate(prediction, item):
        result.update(status="ok", prediction=prediction, exploratory_decimal=True)
    return result


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
