"""원본 이동·갱신으로 제외된 사례와 중단된 판정 묶음을 복구한다."""

from __future__ import annotations

import argparse
import collections
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "retrospective", HERE / "retrospective.py"
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("retrospective script unavailable")
ret = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ret)


def prepare() -> None:
    samples = {
        row["sample_id"]: row
        for row in json.loads((ret.PRIVATE / "samples.json").read_text())
    }
    sources = {
        row["source_id"]: row
        for row in json.loads((ret.PRIVATE / "sources.json").read_text())
    }
    unmatched = json.loads((ret.OUT / "unmatched.json").read_text())
    selected = collections.defaultdict(dict)
    for row in unmatched:
        sample = samples[row["id"]]
        selected[sample["source_id"]][row["id"]] = sample

    recovered = []
    unresolved = []
    prep = ret.load_prep()
    for source_id, targets in selected.items():
        source = sources[source_id]
        path = Path(source["source_path"])
        if not path.is_file():
            path = Path.home() / ".codex/archived_sessions" / path.name
        if not path.is_file():
            unresolved.extend(
                {"id": sid, "reason": "source_missing"} for sid in targets
            )
            continue
        before = path.stat()
        messages = prep.session_messages(
            path, source["provider"], collections.Counter(), {}
        )
        after = path.stat()
        if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
            unresolved.extend(
                {"id": sid, "reason": "source_changed_during_read"} for sid in targets
            )
            continue
        previous = collections.deque(maxlen=5)
        current = []
        seen = set()
        for position, message in enumerate(messages):
            if message["role"] != "user":
                if current:
                    current.append(message)
                continue
            if current:
                previous.append(current)
            state = {
                "previous_context": [item for turn in previous for item in turn],
                "latest_user_input": message["text"],
            }
            current = [message]
            identity = ret.digest(
                json.dumps(state, ensure_ascii=False, sort_keys=True).encode()
            )
            if identity not in targets:
                continue
            seen.add(identity)
            if state != targets[identity]["state"]:
                unresolved.append({"id": identity, "reason": "state_mismatch"})
                continue
            future, coverage = ret.bounded_future(messages[position + 1 :])
            recovered.append(
                {
                    "id": identity,
                    "source_id": source_id,
                    "project_id": targets[identity]["project_id"],
                    "provider": source["provider"],
                    "source_message_index": position,
                    "state": state,
                    "future": future,
                    "future_coverage": coverage,
                }
            )
        unresolved.extend(
            {"id": sid, "reason": "state_not_found"}
            for sid in targets
            if sid not in seen
        )
    recovered.sort(key=lambda row: row["id"])
    ret.write_once(ret.OUT / "recovered/cases.json", recovered)
    ret.write_once(
        ret.OUT / "recovered/prepare-summary.json",
        {
            "ts_utc": ret.now(),
            "requested": len(unmatched),
            "recovered": len(recovered),
            "unresolved": unresolved,
            "method_sha256": ret.digest(Path(__file__).read_bytes()),
        },
    )
    print(json.dumps({"recovered": len(recovered), "unresolved": len(unresolved)}))


def prepare_retry() -> None:
    cases = json.loads((ret.OUT / "cases.json").read_text())
    jobs = ret.batches(cases)
    reserved = {
        json.loads(line)["batch"]
        for line in (ret.OUT / "calls.jsonl").read_text().splitlines()
    }
    missing = sorted(
        index
        for index in reserved
        if not (ret.OUT / "raw" / f"{index:04d}.json").exists()
        or json.loads((ret.OUT / "raw" / f"{index:04d}.json").read_text())["status"]
        != "ok"
    )
    ret.write_once(
        ret.OUT / "retry/cases.json",
        [row for index in missing for row in jobs[index]],
    )
    ret.write_once(ret.OUT / "retry/original-batches.json", missing)
    print(json.dumps({"retry_batches": missing}))


def label(group: str) -> None:
    ret.OUT = ret.OUT / group
    ret.seal()
    ret.label(None)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("prepare", "prepare-retry", "label"))
    parser.add_argument("--group", choices=("recovered", "retry"))
    args = parser.parse_args()
    if args.action == "prepare":
        prepare()
    elif args.action == "prepare-retry":
        prepare_retry()
    elif args.group is None:
        parser.error("--group is required for label")
    else:
        label(args.group)


if __name__ == "__main__":
    main()
