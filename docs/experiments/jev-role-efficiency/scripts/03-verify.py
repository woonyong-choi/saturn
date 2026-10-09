"""표본 누락·원문 변경·선별 예산·정답 누출을 검사한다."""

from __future__ import annotations

import importlib.util

from runtime import PRIVATE, PUBLIC, ROOT, digest, read, safe_state


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def main() -> None:
    seal = read(PRIVATE / "collection-seal.json")
    for name, value in seal["files"].items():
        require(digest(ROOT / name) == value, "protocol changed: " + name)
    for line in (PUBLIC / "data/SHA256SUMS").read_text().splitlines():
        expected, name = line.split("  ", 1)
        require(digest(PRIVATE / name) == expected, "raw hash mismatch")
    summary = read(PUBLIC / "results/summary.json")
    require(summary["collection"]["outcome_rows"] == 106, "outcome count mismatch")
    require(summary["collection"]["missing_outcomes"] == 0, "missing outcomes")
    cases = read(PRIVATE / "cases.json")
    for case in cases["context"]:
        selection = read(PRIVATE / ("context-selection-" + case["id"] + ".json"))
        for policy, ids in selection["selections"].items():
            require(len(set(ids)) == len(ids), "duplicate selected ID")
            require(
                len(ids) == (12 if policy == "full" else 3), "selection size mismatch"
            )
        require(
            all(len(b["text"].encode()) == 512 for b in case["blocks"]),
            "unequal block size",
        )
    for p in (PRIVATE / "raw").glob("*.json"):
        raw = read(p)
        if raw["kind"] == "jev":
            safe_state(__import__("json").loads(raw["request"]["state"]))
    try:
        safe_state({"nested": {"expected": "leak"}})
    except ValueError:
        pass
    else:
        raise RuntimeError("leakage negative control accepted")
    spec = importlib.util.spec_from_file_location(
        "analysis", PUBLIC / "scripts/02-analyze.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    require(
        not module.expression_passes(
            {"expression": '__import__("os").system("true")'}, [(1, 1)]
        ),
        "unsafe expression accepted",
    )
    require(
        not module.expression_passes({"expression": "x"}, [(1, 2)]),
        "wrong answer accepted",
    )
    print("verified raw hashes, protocol, 106 outcomes, budgets and negative controls")


if __name__ == "__main__":
    main()
