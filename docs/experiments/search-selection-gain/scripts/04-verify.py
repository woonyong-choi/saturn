"""표본 독립성, 해시, 계약, 재계산, 두 번 분석의 일치, 주입 결함 거절을 확인한다."""

from __future__ import annotations

import copy
import importlib.util
import json
import subprocess
import sys

import fixtures
import selection
import trials
from lock import LOCK
from runtime import CALLER, PRIVATE, PUBLIC, contract, read

_p = importlib.util.spec_from_file_location("process", PUBLIC / "scripts/02-process.py")
process = importlib.util.module_from_spec(_p)
_p.loader.exec_module(process)


def expect_reject(name: str, fn) -> None:
    try:
        fn()
    except (contract.ContractError, RuntimeError, ValueError, KeyError):
        return
    raise SystemExit("defect not rejected: " + name)


def main() -> None:
    cases = read(PRIVATE / "cases.json")
    sealed = [c for c in cases if c["split"] == "sealed"]
    fixtures.check(cases, prior=set())
    tau = read(LOCK)["tau"]
    provs = process.providers(cases)
    raw = trials.load_raw(PRIVATE / "raw")
    recorded = [json.loads(s) for s in (PRIVATE / "trials.jsonl").read_text().splitlines()]
    trials.check_dataset(sealed, provs, recorded, raw, tau)
    # 분석 두 번이 같은 바이트
    target = (PUBLIC / "results" if CALLER == "live" else PRIVATE / "results") / "summary.json"
    first = target.read_bytes()
    subprocess.run([sys.executable, str(PUBLIC / "scripts/03-analyze.py")], check=True, stdout=subprocess.DEVNULL)
    if target.read_bytes() != first:
        raise SystemExit("analysis is not byte-identical")
    # 주입 결함
    def flip():
        t = copy.deepcopy(recorded)
        t[0]["selection"]["ids"] = t[0]["selection"]["ids"][:-1] or ["r999"]
        trials.check_dataset(sealed, provs, t, raw, tau)
    def usage_drop():
        t = copy.deepcopy(recorded)
        del t[0]["usage"][0]["output_tokens"]
        trials.check_dataset(sealed, provs, t, raw, tau)
    def raw_tamper():
        r = dict(raw)
        k = next(iter(r))
        r[k] = r[k] + b" "
        trials.check_dataset(sealed, provs, recorded, r, tau)
    def always_true():
        trials.check_dataset(sealed, provs, recorded, raw, tau, grader=lambda c, v: True)
    def always_false():
        trials.check_dataset(sealed, provs, recorded, raw, tau, grader=lambda c, v: False)
    def wrong_success():
        t = copy.deepcopy(recorded)
        t[0]["check"]["success"] = not t[0]["check"]["success"]
        trials.check_dataset(sealed, provs, t, raw, tau)
    def shared_call():
        t = copy.deepcopy(recorded)
        t[1]["usage"].append(copy.deepcopy(t[0]["usage"][-1]))
        trials.check_dataset(sealed, provs, t, raw, tau)
    def leaked():
        from runtime import safe_state
        safe_state({"expected": 1})
    for name, fn in [("changed selection", flip), ("missing usage key", usage_drop), ("raw tamper", raw_tamper), ("always-true grader", always_true),
                     ("always-false grader", always_false), ("flipped success", wrong_success), ("shared usage call", shared_call), ("answer leak", leaked)]:
        expect_reject(name, fn)
    print("verify ok:", len(recorded), "trials, 8 injected defects rejected")


if __name__ == "__main__":
    main()
