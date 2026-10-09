"""동일 입력인 두 방식의 응답 변동을 탐색한다. 주 분석에서 제외하지 않는다."""

import hashlib
import importlib.util
import json
from pathlib import Path

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
RUN = ROOT / ".local/experiments/recall-structure/formal"


def load(path):
    return json.loads(path.read_text())


def module(path):
    spec = importlib.util.spec_from_file_location(path.stem, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def analyze():
    scoring = module(PUBLIC / "scripts/02-analyze.py")
    original = module(PUBLIC.parent / "real-context-replay/scripts/03-analyze.py")
    jobs = load(RUN / "calls-plan.json")
    questions = load(RUN / "questions.json")
    pairs = []
    for before in [j for j in jobs if j["arm"] == "baseline"]:
        after = next(
            j for j in jobs if j["arm"] == "repaired" and j["id"] == before["id"]
        )
        if any(before[key] != after[key] for key in ["context", "question"]):
            continue
        for provider in ["claude", "codex"]:
            a = scoring.observation(before, provider, questions, original)
            b = scoring.observation(after, provider, questions, original)
            valid = a["valid"] and b["valid"]
            pairs.append(
                dict(
                    provider=provider,
                    trial_id=before["id"],
                    valid=valid,
                    context_sha256=hashlib.sha256(
                        before["context"].encode()
                    ).hexdigest(),
                    question_sha256=hashlib.sha256(
                        before["question"].encode()
                    ).hexdigest(),
                    losses=[
                        k for k in a["checks"] if a["checks"][k] and not b["checks"][k]
                    ]
                    if valid
                    else None,
                    gains=[
                        k for k in a["checks"] if b["checks"][k] and not a["checks"][k]
                    ]
                    if valid
                    else None,
                )
            )
    groups = []
    for provider in ["claude", "codex"]:
        selected = [p for p in pairs if p["provider"] == provider]
        valid = [p for p in selected if p["valid"]]
        groups.append(
            dict(
                provider=provider,
                planned_pairs=len(selected),
                observed_pairs=len(valid),
                complete=len(selected) == len(valid),
                losses=sum(len(p["losses"]) for p in valid),
                gains=sum(len(p["gains"]) for p in valid),
            )
        )
    result = dict(
        scope="exploratory identical-input subset; no exclusions or changed adoption criterion",
        groups=groups,
        pairs=pairs,
    )
    (PUBLIC / "results/identical-input.json").write_text(
        json.dumps(result, indent=2) + "\n"
    )
    print(json.dumps(groups))


if __name__ == "__main__":
    analyze()
