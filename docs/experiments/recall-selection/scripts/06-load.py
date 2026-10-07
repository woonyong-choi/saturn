"""실제 기록과 복제 부하에서 이전·현재 복원 경과 시간을 측정한다."""

import copy
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/recall-selection"


def main() -> None:
    os.umask(0o077)
    target = PRIVATE / "load"
    target.mkdir(exist_ok=False)
    cases = json.loads((PRIVATE / "explicit/cases.json").read_text())
    case = next(c for c in cases if c["id"] == "learning-59")
    queries = json.loads((PRIVATE / "explicit/queries.json").read_text())
    query = next(q for q in queries if q["id"] == "learning-59")
    (target / "queries.json").write_text(json.dumps([query], ensure_ascii=False))
    binaries = {
        "baseline": PRIVATE / "baseline/engine-tests",
        "candidate": PRIVATE / "current-engine-tests",
    }
    (target / "binaries.json").write_text(
        json.dumps(
            {
                name: dict(
                    path=str(path.relative_to(ROOT)),
                    sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                )
                for name, path in binaries.items()
            },
            indent=2,
        )
    )
    records = []
    for multiplier in [1, 10, 100]:
        scaled = copy.deepcopy(case)
        scaled["rows"] = []
        for index in range(multiplier):
            for original in case["rows"]:
                row = copy.deepcopy(original)
                for field in ["seq", "run", "task"]:
                    row[field] += index * 100000
                scaled["rows"].append(row)
        source = target / f"cases-{multiplier}.json"
        source.write_text(json.dumps([scaled], ensure_ascii=False))
        for repeat in range(1, 4):
            for arm, binary in binaries.items():
                trial = f"{arm}-{multiplier}-{repeat}"
                output = target / trial
                env = dict(
                    os.environ,
                    SATURN_REPLAY_INPUT=str(source),
                    SATURN_RECALL_QUERIES=str(target / "queries.json"),
                    SATURN_REPLAY_OUTPUT=str(output),
                )
                command = [str(binary), "export_recalled_evidence", "--ignored"]
                with (target / f"{trial}.log").open("x") as log:
                    started = time.perf_counter()
                    result = subprocess.run(
                        command,
                        cwd=ROOT,
                        env=env,
                        stdout=log,
                        stderr=log,
                        timeout=180,
                        check=False,
                    )
                    elapsed = time.perf_counter() - started
                records.append(
                    dict(
                        trial=trial,
                        arm=arm,
                        multiplier=multiplier,
                        source_rows=len(scaled["rows"]),
                        repeat=repeat,
                        elapsed_s=elapsed,
                        exit_code=result.returncode,
                        command=command,
                    )
                )
    groups = []
    for multiplier in [1, 10, 100]:
        for arm in binaries:
            rows = [
                r for r in records if r["arm"] == arm and r["multiplier"] == multiplier
            ]
            times = [r["elapsed_s"] for r in rows]
            groups.append(
                dict(
                    arm=arm,
                    multiplier=multiplier,
                    source_rows=rows[0]["source_rows"],
                    runs=len(rows),
                    successes=sum(r["exit_code"] == 0 for r in rows),
                    median_s=statistics.median(times),
                    min_s=min(times),
                    max_s=max(times),
                )
            )
    (PUBLIC / "results/load.json").write_text(
        json.dumps(dict(groups=groups, records=records), indent=2) + "\n"
    )
    print(json.dumps(groups, indent=2))


if __name__ == "__main__":
    main()
