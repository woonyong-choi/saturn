"""원응답에서 성공·사용량·선택 근거를 다시 계산한다."""

from __future__ import annotations

import ast
import json
import math
from statistics import median

from runtime import (
    MODELS,
    PRIVATE,
    PUBLIC,
    digest,
    parse_json,
    read,
    response_text,
    write,
)


def result(trial: str) -> dict:
    path = PRIVATE / "raw" / f"{trial}.json"
    if not path.exists():
        return {
            "trial_id": trial,
            "value": None,
            "status": "missing",
            "latency_s": None,
            "cost_usd": None,
            "usage": {},
            "model_reported": None,
        }
    raw = read(path)
    if raw["kind"] == "jev":
        return {
            "trial_id": trial,
            "value": raw.get("response"),
            "status": raw["status"],
            "latency_s": raw["latency_s"],
            "cost_usd": None,
            "usage": raw.get("response", {}).get("usage", {}),
            "model_reported": raw.get("response", {}).get("model"),
        }
    text, meta = response_text(raw)
    usage = meta.get("usage", {})
    return {
        "trial_id": trial,
        "value": parse_json(text),
        "status": raw["status"],
        "latency_s": raw["latency_s"],
        "cost_usd": meta.get("total_cost_usd"),
        "usage": usage,
        "model_reported": list(meta.get("modelUsage", {})) or None,
    }


def expression_passes(value: object, tests: list) -> bool:
    if not isinstance(value, dict) or not isinstance(value.get("expression"), str):
        return False
    expression = value["expression"]
    if len(expression) > 400:
        return False
    try:
        tree = ast.parse(expression, mode="eval")
        allowed = (
            ast.Expression,
            ast.BinOp,
            ast.UnaryOp,
            ast.Add,
            ast.Sub,
            ast.Mult,
            ast.Div,
            ast.FloorDiv,
            ast.Mod,
            ast.USub,
            ast.UAdd,
            ast.Constant,
            ast.Name,
            ast.Load,
            ast.Call,
        )
        for node in ast.walk(tree):
            if not isinstance(node, allowed):
                return False
            if isinstance(node, ast.Name) and node.id not in ("x", "min", "max", "abs"):
                return False
            if isinstance(node, ast.Constant) and (
                type(node.value) is not int or abs(node.value) > 10000
            ):
                return False
            if isinstance(node, ast.Call) and (
                not isinstance(node.func, ast.Name)
                or node.func.id not in ("min", "max", "abs")
                or node.keywords
            ):
                return False
        code = compile(tree, "<expression>", "eval")
        return all(
            eval(
                code, {"__builtins__": {}, "min": min, "max": max, "abs": abs}, {"x": x}
            )
            == expected
            for x, expected in tests
        )
    except (
        SyntaxError,
        ValueError,
        TypeError,
        ArithmeticError,
        NameError,
        RecursionError,
    ):
        return False


def rate(values: list[bool]) -> dict:
    n = len(values)
    k = sum(values)
    if not n:
        return {"k": 0, "n": 0, "rate": None, "wilson95": None}
    p = k / n
    z = 1.959963984540054
    center = (p + z * z / (2 * n)) / (1 + z * z / n)
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / (1 + z * z / n)
    return {
        "k": k,
        "n": n,
        "rate": p,
        "wilson95": [max(0, center - half), min(1, center + half)],
    }


def summarize(values: list[dict]) -> dict:
    times = [r["latency_s"] for r in values if r["latency_s"] is not None]
    costs = [r["cost_usd"] for r in values if r["cost_usd"] is not None]
    return {
        "success": rate([r["success"] for r in values]),
        "latency_median_s": median(times) if times else None,
        "known_cost_usd": sum(costs),
        "unknown_cost_count": len(values) - len(costs),
        "failure_statuses": {
            s: sum(r["status"] == s for r in values)
            for s in sorted({r["status"] for r in values})
        },
    }


def paired(values: list[dict], baseline: list[dict]) -> dict:
    other = {r["case_id"]: r for r in baseline}
    pairs = [(r["success"], other[r["case_id"]]["success"]) for r in values]
    return {
        "n": len(pairs),
        "delta": sum(int(a) - int(b) for a, b in pairs) / len(pairs),
        "treatment_only": sum(a and not b for a, b in pairs),
        "baseline_only": sum(b and not a for a, b in pairs),
    }


def route_rows(cases: list[dict]) -> tuple[list, dict]:
    rows = []
    for case in cases:
        for model in MODELS:
            r = result("route-run-" + case["id"] + "-" + model)
            success = (
                expression_passes(r["value"], case["tests"])
                if "tests" in case
                else r["value"] == case["expected"]
            )
            rows.append(dict(r, case_id=case["id"], condition=model, success=success))
    policies = {}
    for policy in ("baseline", "direct", "decomposed"):
        selected = []
        for d in read(PRIVATE / "routing-decisions.json"):
            r = next(
                r
                for r in rows
                if r["case_id"] == d["id"] and r["condition"] == d[policy]
            )
            selected.append(r)
        policies[policy] = summarize(selected)
        policies[policy]["selected_models"] = [r["condition"] for r in selected]
        policies[policy]["composed_latency_median_s"] = median(
            [
                (r["latency_s"] or 0)
                + (
                    (result("route-jev-" + r["case_id"])["latency_s"] or 0)
                    if policy != "baseline"
                    else 0
                )
                for r in selected
            ]
        )
        baseline_rows = [
            next(
                r
                for r in rows
                if r["case_id"] == d["id"] and r["condition"] == d["baseline"]
            )
            for d in read(PRIVATE / "routing-decisions.json")
        ]
        policies[policy]["paired_vs_baseline"] = paired(selected, baseline_rows)
        policies[policy]["judge_overhead_s"] = (
            sum(result("route-jev-" + c["id"])["latency_s"] or 0 for c in cases)
            if policy != "baseline"
            else 0
        )
        policies[policy]["note"] = (
            "execution row reuse; judge overhead listed separately, not an independently executed policy"
        )
    return rows, {
        "models": {
            m: summarize([r for r in rows if r["condition"] == m]) for m in MODELS
        },
        "policies": policies,
        "decisions": read(PRIVATE / "routing-decisions.json"),
    }


def constraint_rows(cases: list[dict]) -> tuple[list, dict]:
    rows = []
    coverage = []
    defaults = {"language": "en", "format": "prose", "test": "optional"}
    for case in cases:
        selection = read(PRIVATE / ("constraint-selection-" + case["id"] + ".json"))
        expected = {(k, v) for k, v in case["expected"].items() if v != defaults[k]}
        offered = {(c["key"], c["value"]) for c in selection["candidates"]}
        coverage.append(
            {
                "case_id": case["id"],
                "expected_nondefault": len(expected),
                "offered_expected": len(expected & offered),
            }
        )
        for policy, ids in selection["selections"].items():
            r = result("constraint-run-" + case["id"] + "-" + policy)
            pairs = {
                (c["key"], c["value"])
                for c in selection["candidates"]
                if c["id"] in ids
            }
            wrong = sum(case["expected"][k] != v for k, v in pairs)
            rows.append(
                dict(
                    r,
                    case_id=case["id"],
                    condition=policy,
                    success=r["value"] == case["expected"],
                    wrong_selected_values=wrong,
                    selected_expected=len(expected & pairs),
                    expected_nondefault=len(expected),
                    selected_ids=ids,
                )
            )
    return rows, {
        "conditions": {
            p: dict(
                summarize([r for r in rows if r["condition"] == p]),
                wrong_selected_values=sum(
                    r["wrong_selected_values"] for r in rows if r["condition"] == p
                ),
            )
            for p in ("code", "jev", "llm")
        },
        "candidate_coverage": coverage,
        "note": "value correctness is a pilot proxy; provenance and unsupported application require separate source audit",
    }


def context_rows(cases: list[dict]) -> tuple[list, dict]:
    rows = []
    for case in cases:
        selection = read(PRIVATE / ("context-selection-" + case["id"] + ".json"))
        for policy, ids in selection["selections"].items():
            r = result("context-run-" + case["id"] + "-" + policy)
            rows.append(
                dict(
                    r,
                    case_id=case["id"],
                    condition=policy,
                    success=r["value"] == case["expected"],
                    support_recall=len(set(ids) & set(case["support_ids"]))
                    / len(case["support_ids"]),
                    selected_ids=ids,
                    block_bytes=sum(
                        len(b["text"].encode())
                        for b in case["blocks"]
                        if b["id"] in ids
                    ),
                )
            )
    return rows, {
        "conditions": {
            p: dict(
                summarize([r for r in rows if r["condition"] == p]),
                mean_support_recall=sum(
                    r["support_recall"] for r in rows if r["condition"] == p
                )
                / len(cases),
            )
            for p in ("full", "recency", "lexical", "jev")
        }
    }


def main() -> None:
    cases = read(PRIVATE / "cases.json")
    all_rows = []
    summary = {
        "scope": "synthetic exploratory instrumentation pilot; no adoption claim",
        "hypotheses": {"H1": "deferred", "H2": "deferred", "H3": "deferred"},
    }
    for lane, fn in [
        ("routing", route_rows),
        ("constraints", constraint_rows),
        ("context", context_rows),
    ]:
        rows, summary[lane] = fn(cases[lane])
        all_rows.extend(dict(r, lane=lane) for r in rows)
    raw = list(sorted((PRIVATE / "raw").glob("*.json")))
    summary["collection"] = {
        "raw_records": len(raw),
        "outcome_rows": len(all_rows),
        "missing_outcomes": sum(r["status"] == "missing" for r in all_rows),
    }
    write(PUBLIC / "results/summary.json", summary)
    write(PRIVATE / "processed.json", all_rows)
    manifest = "".join(
        digest(p) + "  " + str(p.relative_to(PRIVATE)) + "\n" for p in raw
    )
    (PUBLIC / "data/SHA256SUMS").write_text(manifest)
    print(json.dumps(summary["collection"]))


if __name__ == "__main__":
    main()
