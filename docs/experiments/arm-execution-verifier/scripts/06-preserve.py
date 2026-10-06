"""보존 우선 비교(#540) 명령. plan | seal | collect | process | verify.

verify는 가짜 engine 기록으로 만든 데이터셋에 결함을 하나씩 주입해 모두 거절하는지 확인하고, 실제 원자료가 있으면 같은 검사를 적용한다.
실제 원자료와 봉인 파일은 git이 무시하는 .runtime/preserve에만 둔다.
"""

from __future__ import annotations

import copy
import json
import os
import platform
import subprocess
import sys
from pathlib import Path

import contract
import preserve
import stub
from arm_runtime import PUBLIC, ROOT, load_script

RUN = ROOT / ".runtime/preserve"
RAW, MANIFEST, TRIALS, SUMMARY = RUN / "raw", RUN / "manifest.json", RUN / "trials.jsonl", RUN / "summary.json"
must_fail = load_script(PUBLIC / "scripts/04-verify.py", "arm_verify").must_fail


def experiment_files() -> dict:
    files = [PUBLIC / "design.md", *sorted((PUBLIC / "scripts").glob("*.py"))]
    return {str(f.relative_to(ROOT)): preserve.sha(f.read_bytes()) for f in files}


def git(*args: str, cwd: Path = ROOT) -> str:
    return subprocess.check_output(["git", *args], cwd=cwd, text=True).strip()


def seal() -> None:
    """수집 전에 제품·실험·정책·CLI 버전, seed, 가격표, 상한, 지원 범위를 한 파일로 고정한다. 실험 파일은 커밋된 상태여야 한다."""
    if git("status", "--porcelain", "--", str(PUBLIC / "design.md"), str(PUBLIC / "scripts")):
        raise SystemExit("design and scripts must be committed")
    binary = Path(os.environ.get("ARM_ENGINE_BIN", ""))
    if not binary.is_file():
        raise SystemExit("ARM_ENGINE_BIN must point to the saturn-engine binary")
    if MANIFEST.exists():
        raise SystemExit("already sealed: " + str(MANIFEST))
    prices = json.loads((RUN / "prices.json").read_text())  # USD per million tokens, 사용자가 공급한다. 기본값을 만들지 않는다
    version = lambda cmd: subprocess.check_output([cmd, "--version"], text=True).strip()
    manifest = preserve.build_manifest(
        product=dict(commit=git("rev-parse", "HEAD", cwd=binary.parents[2]), binary_sha256=preserve.sha(binary.read_bytes())),
        experiment=dict(commit=git("rev-parse", "HEAD"), files=experiment_files()),
        cli=dict(claude=version("claude"), codex=version("codex"), os=platform.platform()),
        prices=prices, capabilities=json.loads((RUN / "capabilities.json").read_text()) if (RUN / "capabilities.json").exists() else preserve.default_capabilities(),
    )
    MANIFEST.write_text(json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n")
    print("sealed", preserve.sha(MANIFEST.read_bytes()))


def load_manifest() -> dict:
    manifest = json.loads(MANIFEST.read_text())
    preserve.validate_manifest(manifest)
    preserve.check_files_unchanged(manifest, experiment_files())
    return manifest


def collect() -> None:
    """수집 전 사전 점검. 모든 요건이 맞아야 시작하며, 맞지 않으면 막힘을 나열하고 끝낸다. 이 변경에는 계열 대화를 engine에 재생하는 수집기가 없다."""
    blockers = []
    try:
        manifest = load_manifest()
    except (OSError, ValueError, KeyError, RuntimeError) as error:
        manifest, blockers = None, ["manifest: " + str(error)]
    if manifest:
        cells = preserve.plan(manifest["capabilities"])
        runnable = [c for c in cells if c["status"] == "planned"]
        if not runnable:
            blockers.append("no executable cell in the sealed plan")
        blockers.append(f"online driver replaying the six families is not implemented ({len(runnable)} cells would be executable)")
    for name in ("SATURN_JUDGE_KEY", "ARM_ENGINE_BIN"):
        if not os.environ.get(name):
            blockers.append("missing " + name)
    print("collection blocked:\n- " + "\n- ".join(blockers))
    raise SystemExit(1)


def load_raw() -> dict:
    return {f"raw/{p.name}": p.read_bytes() for p in sorted(RAW.glob("*.json"))}


def build_all(manifest: dict, raws: dict) -> tuple:
    cells, fixs = preserve.plan(manifest["capabilities"]), preserve.fixtures()
    by_cell = {c["cell_id"]: c for c in cells}
    trials = []
    for name, data in raws.items():
        cell = by_cell[Path(name).stem]
        trials.append(preserve.build_trial(cell, fixs[cell["family_id"]], json.loads(data), name, preserve.sha(data), manifest["prices"]))
    return cells, trials, fixs


def summary_bytes(cells: list, trials: list) -> bytes:
    return (json.dumps(preserve.analyze(cells, trials), ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode()


def process() -> None:
    manifest = load_manifest()
    cells, trials, _ = build_all(manifest, load_raw())
    TRIALS.write_text("".join(json.dumps(t, ensure_ascii=False, sort_keys=True) + "\n" for t in trials))
    SUMMARY.write_bytes(summary_bytes(cells, trials))
    print(f"processed {len(trials)} trials of {len(cells)} planned cells")


def check_repeat(analyze, *args) -> bytes:
    first = analyze(*args)
    if first != analyze(*args):
        raise contract.ContractError("analysis is not byte-identical")
    return first


# 가짜 기록으로 만든 데이터셋과 결함 주입 ------------------------------------------------

SIM_PRICES = {m: dict(input=1.0, cache_write=1.0, cache_read=1.0, output=1.0) for m in preserve.MODELS}
SIM_PRICES["jev"].update(cache_write=None, cache_read=None)


def simulated(caps: dict, edit: dict | None = None) -> tuple:
    """edit: cell_id -> 기록을 바꾸는 함수. 바꾼 기록으로 trial을 다시 만들어 해시까지 일관된 자료를 얻는다."""
    cells, fixs = preserve.plan(caps), preserve.fixtures()
    raws, trials = {}, []
    for index, cell in enumerate(c for c in cells if c["status"] == "planned"):
        record = stub.engine_record(cell, fixs[cell["family_id"]], index)
        if edit and cell["cell_id"] in edit:
            edit[cell["cell_id"]](record)
        name = f"raw/{cell['cell_id']}.json"
        raws[name] = json.dumps(record, sort_keys=True).encode()
        trials.append(preserve.build_trial(cell, fixs[cell["family_id"]], record, name, preserve.sha(raws[name]), SIM_PRICES))
    return cells, trials, raws, fixs


def first(trials: list, arm: str, where=lambda t: True) -> dict:
    for t in trials:
        if t["arm"] == arm and where(t):
            return t
    raise SystemExit(f"no {arm} trial to inject into")


def refresh(trial: dict, raws: dict, name: str) -> None:
    trial["raw_files"] = [dict(name=name, sha256=preserve.sha(raws[name]))]
    trial["raw_sha256"] = trial["raw_files"][0]["sha256"]


def mutate_manifest(change) -> dict:
    manifest = preserve.build_manifest(
        product=dict(commit="0" * 40, binary_sha256="0" * 64), experiment=dict(commit="0" * 40, files={"x": "y"}),
        cli=dict(claude="sim", codex="sim", os="sim"), prices=copy.deepcopy(SIM_PRICES), capabilities=copy.deepcopy(ALL_CAPS))
    change(manifest)
    return manifest


ALL_CAPS = {k: dict(supported=True, evidence="simulation only") for k in preserve.default_capabilities()}


def injections() -> None:
    cells, trials, raws, fixs = simulated(ALL_CAPS)
    prices = SIM_PRICES
    check = lambda t=None, r=None, c=None: lambda: preserve.check_dataset(c or cells, t if t is not None else trials, r or raws, fixs, prices)
    check()()
    assert len(cells) == 96 and all(c["status"] == "planned" for c in cells)
    assert any(t["status"] == "delivery_unknown" for t in trials) and any(t["preserve"]["cost_usd"] is None for t in trials)
    some = lambda arm: next(c["cell_id"] for c in cells if c["arm"] == arm and c["source"] != c["target"])
    name = lambda cid: f"raw/{cid}.json"

    def semantic(label: str, cell_id: str, edit) -> None:
        """기록 자체가 잘못된 경우: 해시와 trial을 일관되게 다시 만들어도 거절해야 한다."""
        _, t, r, _ = simulated(ALL_CAPS, {cell_id: edit})
        must_fail(label, check(t, r))

    def drop_user(record: dict) -> None:
        db, user = record["db"], next(i for i in record["db"]["items"] if i["zone"] == "User")
        db["items"].remove(user)
        record["sent_body"] = record["sent_body"].replace("User: " + next(i["text"] for i in db["inputs"] if preserve.sha(i["text"]) == user["body_hash"]), "")
        db["packets"][0]["body_hash"] = preserve.sha(record["sent_body"])

    def swap_role(record: dict) -> None:
        item = next(i for i in record["db"]["items"] if i["zone"] == "User")
        item["zone"] = "Assistant"

    def swap_order(record: dict) -> None:
        items = record["db"]["items"]
        items[0], items[1] = items[1], items[0]

    def edit_sent(record: dict) -> None:
        record["sent_body"] = record["sent_body"].replace("User:", "Usr:", 1)

    def leak(record: dict) -> None:
        record["db"]["inputs"][0]["text"] += " " + fixs[record["name"].split("-")[0]]["hidden"]["canary"]

    def resend(record: dict) -> None:
        packet = record["db"]["packets"][0]
        packet["state"] = "Unknown"
        record["db"]["packets"].append(dict(packet, id=2, attempt=2, reduced_from=1, state="Sent"))

    def selector_dropped(record: dict) -> None:
        record["db"]["packets"][0].update(requested_selector="rank", actual_selector="rank")

    def silent_fallback(record: dict) -> None:
        record["db"]["packets"][0].update(actual_selector="rank", selection_fallback=None)

    def other_safety(record: dict) -> None:
        record["overrides"] = [[k, "100" if k == preserve.SAFETY_KEY else v] for k, v in record["overrides"]]

    j, r = some("J"), some("R")
    semantic("protected body omitted", r, drop_user)
    semantic("user and assistant roles swapped", r, swap_role)
    semantic("protected order changed", r, swap_order)
    semantic("sent body differs from its hash", r, edit_sent)
    semantic("grading canary inside delivered material", r, leak)
    semantic("workdir keeps hidden file", r, lambda rec: rec["workdir_files"].append("hidden.json"))
    semantic("resent after unknown delivery", r, resend)
    semantic("Jev selector request not applied", j, selector_dropped)
    semantic("fallback without a recorded reason", j, silent_fallback)
    semantic("arms differ in send limit", j, other_safety)
    semantic("Jev arm without the selector setting", j, lambda rec: rec.update(overrides=[o for o in rec["overrides"] if o[0] != preserve.PACKET_OVERRIDE[0]]))
    for label, grader in (("always-success grader", lambda *_: [True] * 3), ("always-fail grader", lambda *_: [False] * 3)):
        must_fail(label, lambda g=grader: preserve.grader_selftest(g, fixs))

    def tamper(label: str, apply) -> None:
        t = copy.deepcopy(trials)
        apply(t, copy.deepcopy(raws))
        must_fail(label, check(t))

    null_cost = lambda t: t["preserve"]["cost_usd"] is None
    not_applied = lambda t: not t["preserve"]["selector"]["applied"]
    tamper("usage field missing", lambda t, _: first(t, "R")["usage"][0].pop("output_tokens"))
    tamper("usage entries missing", lambda t, _: first(t, "R", lambda x: x["status"] == "ok").update(usage=[]))
    tamper("child usage counted inside parent", lambda t, _: _child(first(t, "R")))
    tamper("duplicate child usage id", lambda t, _: first(t, "R")["usage"].append(dict(first(t, "R")["usage"][0], role="child", parent_call_id=first(t, "R")["usage"][0]["call_id"])))
    tamper("usage call shared between trials", lambda t, _: first(t, "J")["usage"].append(dict(first(t, "R")["usage"][0])))
    tamper("missing cost shown as zero", lambda t, _: next(x for x in t if null_cost(x))["preserve"].update(cost_usd=0.0))
    tamper("delivery unknown counted as success", lambda t, _: first(t, "R", lambda x: x["status"] == "delivery_unknown").update(check=dict(grader="hidden-v1", items=[True] * 3, success=True)))
    tamper("selector flag flipped", lambda t, _: first(t, "J", not_applied)["preserve"]["selector"].update(applied=True))
    tamper("path kind rewritten", lambda t, _: first(t, "R")["preserve"]["path"].update(actual="same_session"))
    tamper("success flag flipped", lambda t, _: next(x for x in t if not x["check"]["success"])["check"].update(success=True))
    tamper("provider session reused", lambda t, _: first(t, "J")["preserve"]["packet"].update(provider_session=first(t, "R")["preserve"]["packet"]["provider_session"]))
    name_r = name(r)
    must_fail("raw bytes changed", check(r=dict(raws, **{name_r: raws[name_r] + b" "})))
    changed = dict(raws, **{name_r: json.dumps(dict(json.loads(raws[name_r]), answers=["x", "y", "z"]), sort_keys=True).encode()})
    t = copy.deepcopy(trials)
    refresh(next(x for x in t if x["decision_id"] == r), changed, name_r)
    must_fail("response changed and hash refreshed", check(t, changed))

    # F와 N은 지원하지 않는 동안 다른 조건으로 바꿔 채우면 안 된다. 지원 범위 밖 칸의 trial을 거절한다.
    narrow = {k: dict(v, supported=k not in ("full_packet_control", "native_same_session_observable")) for k, v in ALL_CAPS.items()}
    unsupported = preserve.plan(narrow)
    assert {c["reason"] for c in unsupported if c["status"] != "planned"} == {"no_full_packet_control", "no_native_same_session_observable"}
    must_fail("F or N run substituted for an unsupported cell", check(c=unsupported))
    f_cell = next(c["cell_id"] for c in cells if c["arm"] == "F")
    _, high, high_raw, _ = simulated(ALL_CAPS, {f_cell: other_safety})
    must_fail("F run with silently high safety percent", check(c=unsupported, t=[x for x in high if x["arm"] == "F"], r=high_raw))

    # 봉인: 값이 비거나 봉인 뒤 파일이 바뀌면 시작하지 않는다.
    for label, change in (
        ("manifest without seed", lambda m: m.pop("seed")), ("manifest without price table", lambda m: m.update(prices={})),
        ("manifest without Jev price", lambda m: m["prices"].pop("jev")), ("manifest without CLI version", lambda m: m["cli"].update(codex=None)),
        ("manifest without caps", lambda m: m.update(caps=None)), ("manifest without product commit", lambda m: m["product"].update(commit="")),
        ("capability claimed without evidence", lambda m: m["capabilities"]["full_packet_control"].update(evidence=None)),
        ("plan edited after sealing", lambda m: m.update(plan_hash="0" * 64)),
    ):
        must_fail(label, lambda c=change: validate_changed(c))
    sealed = mutate_manifest(lambda m: None)
    must_fail("experiment file changed after sealing", lambda: preserve.check_files_unchanged(sealed, {"x": "z"}))

    # 두 번 분석한 결과의 바이트 일치. 매번 다른 값을 내는 분석기는 거절한다.
    first_bytes = check_repeat(summary_bytes, cells, trials)
    ticks = iter(range(10 ** 6))
    must_fail("analysis differs between runs", lambda: check_repeat(lambda: str(next(ticks)).encode()))
    summary = json.loads(first_bytes)
    assert summary["planned"] == 96 and summary["collected"] == 96
    print("simulated dataset: 96 cells, injected defects rejected")


def validate_changed(change) -> None:
    manifest = mutate_manifest(lambda m: None)
    change(manifest)
    preserve.validate_manifest(manifest)


def _child(trial: dict) -> None:
    parent = trial["usage"][0]
    parent["includes_children"] = True
    trial["usage"].append(dict(parent, call_id="child-1", role="child", parent_call_id=parent["call_id"], includes_children=False))


def verify() -> None:
    injections()
    cells = preserve.plan(preserve.default_capabilities())
    reasons = {r: [c["reason"] for c in cells].count(r) for r in sorted({c["reason"] for c in cells if c["reason"]})}
    print("current plan:", json.dumps(reasons, sort_keys=True), "executable", sum(c["status"] == "planned" for c in cells), "of", len(cells))
    if not any(RAW.glob("*.json")) if RAW.exists() else True:
        print("no collected data: 0 of", len(cells), "cells measured")
        return
    manifest = load_manifest()
    cells, trials, fixs = build_all(manifest, load_raw())
    preserve.check_dataset(cells, trials, load_raw(), fixs, manifest["prices"])
    check_repeat(summary_bytes, cells, trials)
    print("collected data verified:", len(trials))


if __name__ == "__main__":
    {"plan": lambda: print(json.dumps(preserve.analyze(preserve.plan(preserve.default_capabilities()), []), indent=2, sort_keys=True)),
     "seal": seal, "collect": collect, "process": process, "verify": verify}[sys.argv[1]]()
