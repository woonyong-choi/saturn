"""종료된 과제의 실제 Python 값과 보존된 정적 채점을 대조한다."""

from __future__ import annotations

import gzip
import json
import subprocess

import collect


def main() -> None:
    rows = []
    files = sorted((collect.EXP / "data/raw").glob("*.gz")) + sorted(
        (collect.EXP / "data/followup").glob("*.gz")
    )
    for path in files:
        raw = json.load(gzip.open(path, "rt"))
        if raw["seed"] not in collect.SEEDS or "f3" not in raw["grades"]:
            continue
        runtime = "r587f" if raw.get("phase") == "followup" else "r587"
        provider = "a" if raw["provider"] == "claude" else "x"
        name = f"{provider}{raw['seed']}{collect.ARMS.index(raw['arm'])}"
        workdir = collect.REPO / ".runtime" / runtime / name / "practice"
        script = (
            "import json; from app import config,report; "
            "print(json.dumps({name:getattr(module,name) "
            "for module,names in [(config,['REQUEST_HEADER','TIMEOUT_SECONDS','RETRIES']),"
            "(report,['SLOW_ENDPOINT'])] for name in names}))"
        )
        result = subprocess.run(
            ["python3", "-c", script],
            cwd=workdir,
            text=True,
            capture_output=True,
            timeout=30,
            check=True,
        )
        actual = json.loads(result.stdout.strip().splitlines()[-1])
        expected = {k: raw["grades"]["f3"]["values"][k] for k in actual}
        if actual != expected:
            raise ValueError(
                "saved grading differs from actual module: " + raw["trial"]
            )
        rows.append({"trial": raw["trial"], "runtime_matches_saved_grade": True})
    (collect.EXP / "results/final-state-verification.json").write_text(
        json.dumps(rows, sort_keys=True, indent=2) + "\n"
    )
    print("actual module values match saved grades", len(rows))


if __name__ == "__main__":
    main()
