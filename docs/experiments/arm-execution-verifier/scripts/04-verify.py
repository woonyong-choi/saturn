"""표본 독립성, 원자료, 계약, 선택 재계산, 채점, 사용량을 검사하고 주입한 결함을 반드시 거절하는지 확인한다."""

from __future__ import annotations

import copy
import json

import arms
import contract
import sample
import trials
from arm_runtime import PRIVATE, PUBLIC, RESULTS, load_script, read

REJECTED = (RuntimeError, ValueError)


def must_fail(name: str, action) -> None:
    try:
        action()
    except REJECTED as error:
        print("rejected:", name, "-", str(error)[:70])
        return
    raise SystemExit("verifier accepted an injected defect: " + name)


def first(rows: list[dict], arm: str, fallback: bool | None = None, status: str = "ok") -> dict:
    for t in rows:
        if t["arm"] == arm and t["status"] == status and (fallback is None or t["selection"]["fallback"] == fallback):
            return t
    raise SystemExit(f"no {arm} trial to inject into")


def fix_hashes(trial: dict, raw: dict) -> None:
    for f in trial["raw_files"]:
        f["sha256"] = trials.sha(raw[f["name"]])
    trial["raw_sha256"] = trials.sha(
        "\n".join(f"{f['name']}:{f['sha256']}" for f in trial["raw_files"]).encode()
    )


def injections(cases: list[dict], rows: list[dict], raw: dict) -> None:
    def run(mutated_rows, mutated_raw=raw, grader=arms.grade):
        return lambda: trials.check_dataset(cases, mutated_rows, mutated_raw, grader)

    # 잘못된 선별: 조건마다 선택 ID 하나를 후보 안의 다른 블록으로 바꾼다.
    for arm, fallback in (("code", None), ("llm", False), ("jev", False)):
        bad = copy.deepcopy(rows)
        t = first(bad, arm, fallback)
        spare = next(b for b in arms.block_ids(next(c for c in cases if c["task_id"] == t["task_id"])) if b not in t["selection"]["ids"])
        t["selection"]["ids"][-1] = spare
        must_fail(f"wrong selection ({arm})", run(bad))
    bad = copy.deepcopy(rows)
    first(bad, "jev", True)["selection"]["fallback"] = False
    must_fail("fallback reported as applied", run(bad))

    # 항상 성공·항상 실패하는 채점기, 뒤집힌 성공 표시.
    must_fail("always-success grader", run(trials.build_trials(cases, raw, lambda *_: True), grader=lambda *_: True))
    must_fail("always-fail grader", run(trials.build_trials(cases, raw, lambda *_: False), grader=lambda *_: False))
    bad = copy.deepcopy(rows)
    t = next(t for t in bad if not t["check"]["success"])
    t["check"]["success"] = True
    must_fail("flipped success flag", run(bad))

    # 사용량 누락: 키 누락, 호출 목록 비움, 필드 이름 바꿈.
    bad = copy.deepcopy(rows)
    first(bad, "full")["usage"][0].pop("output_tokens")
    must_fail("usage field missing", run(bad))
    bad = copy.deepcopy(rows)
    first(bad, "code")["usage"] = []
    must_fail("usage entries missing", run(bad))

    # 중복 child 사용량: 같은 호출 ID, 합계에 이미 포함된 parent의 child, 다른 trial과 공유.
    bad = copy.deepcopy(rows)
    t = first(bad, "full")
    t["usage"].append(dict(t["usage"][0], role="child", parent_call_id=t["usage"][0]["call_id"]))
    must_fail("duplicate child usage id", run(bad))
    bad = copy.deepcopy(rows)
    t = first(bad, "full")
    t["usage"][0]["includes_children"] = True
    t["usage"].append(dict(t["usage"][0], call_id="child-1", role="child", parent_call_id=t["usage"][0]["call_id"], includes_children=False))
    must_fail("child usage already inside parent", run(bad))
    bad = copy.deepcopy(rows)
    first(bad, "full")["usage"].append(
        dict(first(bad, "code")["usage"][0])
    )
    must_fail("usage call shared between trials", run(bad))
    ok = copy.deepcopy(first(rows, "full")["usage"])
    ok.append(dict(ok[0], call_id="child-ok", role="child", parent_call_id=ok[0]["call_id"]))
    merged = contract.merge_usage(ok)
    assert merged["calls"] == 2 and merged["input_tokens"] == 2 * ok[0]["input_tokens"], "valid child usage must sum once"

    # 원자료 변조: 바이트 변경, 응답을 바꾸고 해시까지 맞춘 경우, 정답 누출.
    name = first(rows, "full")["raw_files"][-1]["name"]
    tampered = dict(raw, **{name: raw[name] + b" "})
    must_fail("raw bytes changed", run(rows, tampered))
    bad = copy.deepcopy(rows)
    t = first(bad, "full")
    record = json.loads(raw[name])
    envelope = json.loads(record["stdout"])
    envelope["result"] = json.dumps({"port": 1})
    record["stdout"] = json.dumps(envelope)
    tampered = dict(raw, **{name: json.dumps(record).encode()})
    fix_hashes(t, tampered)
    must_fail("response changed and hash refreshed", run(bad, tampered))
    jev_name = next(n for n in raw if n.startswith("raw/select-jev-"))
    record = json.loads(raw[jev_name])
    state = json.loads(record["request"]["state"])
    state["expected"] = {"port": 1}
    record["request"]["state"] = json.dumps(state)
    must_fail("expected answer inside Jev request", lambda: trials.check_leakage(dict(raw, **{jev_name: json.dumps(record).encode()})))

    # 온라인 입력은 engine이 정하는 식별자가 필요하다.
    must_fail("online trial without run identifiers", lambda: contract.validate_trial(rows[0], online=True))


def main() -> None:
    cases = read(PRIVATE / "cases.json")
    sample.check_independence(cases, sample.prior_block_texts())
    rows = [json.loads(s) for s in (PRIVATE / "trials.jsonl").read_text().splitlines()]
    raw = trials.load_raw()
    for line in (PRIVATE / "SHA256SUMS").read_text().splitlines():
        digest, name = line.split("  ", 1)
        if trials.sha(raw[name]) != digest or name not in raw:
            raise SystemExit("raw hash mismatch: " + name)
    trials.check_dataset(cases, rows, raw)
    analyze = load_script(PUBLIC / "scripts/03-analyze.py", "arm_analyze")
    outputs = []
    for _ in range(2):
        analyze.main()
        outputs.append((RESULTS / "summary.json").read_bytes())
    if outputs[0] != outputs[1]:
        raise SystemExit("analysis is not repeatable")
    summary = json.loads(outputs[0])
    for split, part in summary["splits"].items():
        expected = sum(c["split"] == split for c in cases)
        for arm, s in part["arms"].items():
            if s["trials"] != expected or sum(s["statuses"].values()) != expected:
                raise SystemExit(f"silent exclusion in {split}/{arm}")
    injections(cases, rows, raw)
    print(f"verified {len(rows)} trials, repeatable analysis and injected defects")


if __name__ == "__main__":
    main()
