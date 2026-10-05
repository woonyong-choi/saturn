"""개발 계열의 Jev 응답으로 확률 기준 tau를 고정한다. 확인 평가 자료는 열지 않는다."""

from __future__ import annotations

import json
import subprocess

import fixtures
import selection
from runtime import CALLER, PRIVATE, PUBLIC, ROOT, read, rows, write

CANDIDATES = (0.3, 0.5, 0.7, 0.9)
LOCK = (PUBLIC if CALLER == "live" else PRIVATE) / "protocol-lock.json"


def recall(case: dict, ids: list[str]) -> float | None:
    if not case["support_ids"]:
        return None
    return sum(s in ids for s in case["support_ids"]) / len(case["support_ids"])


def main() -> None:
    dev = [c for c in fixtures.build() if c["split"] == "dev"]
    table = {}
    for tau in CANDIDATES:
        recalls, undecided, sizes, none_selected = [], 0, [], 0
        for c in dev:
            recs = [read(PRIVATE / "raw" / f"jev-{c['task_id']}-p{n}.json") for n in range(len(selection.jev_pieces(c)))]
            probs = selection.probabilities(recs, c)
            if probs is None:
                raise RuntimeError("dev jev response invalid: " + c["task_id"])
            sel = selection.select("jev", c, fixtures.BUDGET_BYTES, tau, recs)
            undecided += sel["applied"] != "jev"
            r = recall(c, sel["ids"])
            if r is not None:
                recalls.append(r)
            else:
                none_selected += sel["applied"] == "jev"
            sizes.append(len(sel["ids"]))
        table[str(tau)] = dict(
            support_recall=sum(recalls) / len(recalls), undecided=undecided,
            mean_selected=sum(sizes) / len(sizes), none_stratum_selected=none_selected,
        )
    # 지원 근거 재현율이 가장 큰 기준값, 같으면 더 큰 기준값(더 엄격한 쪽)
    tau = max(CANDIDATES, key=lambda t: (round(table[str(t)]["support_recall"], 6), t))
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    write(LOCK, dict(tau=tau, budget_bytes=fixtures.BUDGET_BYTES, selected_from="dev jev responses, support recall",
                     table=table, dev_tasks=len(dev), commit_when_locked=commit))
    print("tau", tau, json.dumps(table))


if __name__ == "__main__":
    main()
