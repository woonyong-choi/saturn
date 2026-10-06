"""보존 우선 비교(#540) 명령. plan | seal | collect | process | verify.

verify는 가짜 engine 기록으로 만든 데이터셋에 결함을 하나씩 주입해 모두 거절하는지 확인하고, 실제 원자료가 있으면 같은 검사를 적용한다.
실제 원자료와 봉인 파일은 git이 무시하는 .runtime/preserve에만 둔다.
"""

from __future__ import annotations

import contextlib
import copy
import io
import json
import os
import platform
import subprocess
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch

import contract
import family
import ledger
import preserve
import stub
from arm_runtime import PUBLIC, ROOT, load_script

RUN = ROOT / ".runtime/preserve"
RAW, MANIFEST, TRIALS, SUMMARY = RUN / "raw", RUN / "manifest.json", RUN / "trials.jsonl", RUN / "summary.json"
LEDGER = RUN / "ledger"  # 시도 장부. 소유자만 읽고 쓴다
LIVE = RUN / "l"  #스냅샷과 시험 홈. 소켓 경로 길이 때문에 짧게 둔다
LOGIN_CHECKS = {"claude": ["claude", "auth", "status"], "codex": ["codex", "login", "status"]}
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
    def version(cmd: str) -> str:
        return subprocess.check_output([cmd, "--version"], text=True).strip()
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
    """실제 수집. 사전 점검이 모두 맞아야 시작하며, 맞지 않으면 provider를 부르지 않고 막힘을 나열한 뒤 끝낸다.
    봉인된 예정표에서 실행 가능한 칸(다른 provider로 가는 R·J)만 돌린다. 지원하지 않는 칸은 다른 조건이나 stub으로 채우지 않는다.
    이미 원자료가 있는 칸은 다시 돌리지 않는다. 결과 불명이어도 자동 재시도는 없다."""
    manifest, blockers = None, []
    try:
        manifest = load_manifest()
    except (OSError, ValueError, KeyError, RuntimeError) as error:
        blockers.append("manifest: " + str(error))
    cells = preserve.plan(manifest["capabilities"] if manifest else preserve.default_capabilities())
    runnable = [c for c in cells if c["status"] == "planned"]
    if manifest and not runnable:
        blockers.append("no executable cell in the sealed plan")
    blockers += log_read_blockers(runnable, preserve.fixtures())
    if any(c["arm"] in ("F", "N") or c["source"] == c["target"] for c in runnable):
        blockers.append("this collector replays only cross-provider R and J cells; the sealed plan has others")
    binary = Path(os.environ.get("ARM_ENGINE_BIN", ""))
    if not binary.is_file():
        blockers.append("ARM_ENGINE_BIN must point to the saturn-engine binary")
    elif manifest and preserve.sha(binary.read_bytes()) != manifest["product"]["binary_sha256"]:
        blockers.append("engine binary differs from the sealed binary_sha256")
    if not os.environ.get("SATURN_KEY"):
        blockers.append("missing SATURN_KEY (router key the engine reads)")
    for name, command in LOGIN_CHECKS.items():
        try:
            ok = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60).returncode == 0
        except (OSError, subprocess.SubprocessError):
            ok = False
        if not ok:
            blockers.append(f"{name} CLI is missing or not logged in ({' '.join(command)})")
    if family.socket_path_problem(LIVE):
        blockers.append(family.socket_path_problem(LIVE))
    if blockers:
        reasons = {r: [c["reason"] for c in cells].count(r) for r in sorted({c["reason"] for c in cells if c["reason"]})}
        print("collection blocked:\n- " + "\n- ".join(blockers) + f"\nunsupported cells stay in the denominator: {json.dumps(reasons, sort_keys=True)}")
        raise SystemExit(1)
    run_cells(runnable, manifest["caps"])


TRIAL_PACKET_ATTEMPTS = 2  # 인계 패킷 시도 상한: 처음 한 번과 보내지 않음이 확정된 뒤의 축소 재시도 한 번


def snapshot_reserve(fix: dict) -> dict:
    """source 스냅샷 예약치: 입력과 첫 session 패킷 최대 두 시도, 입력마다 관계 판단 한 번."""
    n = len(fix["visible"]["users"])
    return dict(inputs=n + TRIAL_PACKET_ATTEMPTS, jev=n)


def trial_reserve(cell: dict, fix: dict) -> dict:
    """시험 하나의 보수적 상한: 후속 요청마다 provider 한 번과 관계 판단 한 번, 패킷 시도마다 provider 한 번, J 조건은 선별 Jev를 패킷 시도마다 한 번 더."""
    n = len(fix["visible"]["followups"])
    return dict(inputs=n + TRIAL_PACKET_ATTEMPTS, jev=n + (TRIAL_PACKET_ATTEMPTS if cell["arm"] == "J" else 0))


def snapshot_label(cell: dict, fix: dict) -> str:
    return f"{preserve.FAMILY_SEEDS.index(fix['visible']['seed'])}{'a' if cell['source'] == 'claude' else 'b'}"


def write_raw(path: Path, record: dict) -> None:
    """임시 파일에 쓰고 디스크에 닿은 뒤 이름을 바꾼다. 쓰다 멈춘 파일이 완료된 원자료처럼 보이지 않는다."""
    partial = path.with_name(path.name + ".partial")
    fd = os.open(partial, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as out:
        out.write(json.dumps(record, ensure_ascii=False, sort_keys=True))
        out.flush()
        os.fsync(out.fileno())
    os.replace(partial, path)


def log_read_blockers(cells: list, fixs: dict) -> list:
    """고정 입력이 로그 읽기를 요청하지 않는 계열은 재생할 수 없다. 수집 전에 막힘으로 올리고 데이터를 바꾸거나 합성 결과로 메우지 않는다."""
    bad = sorted({c["family_id"] for c in cells if not family.log_read_requested(fixs[c["family_id"]])})
    return [f"family {f} never asks to read {family.LOG_FILE}; replay unsupported" for f in bad]


def run_cells(runnable: list, caps: dict | None = None) -> None:
    """칸마다 provider를 부르기 전에 장부에 시도를 남긴다. 장부에 시작이 있는 칸과 source 스냅샷은 원자료나 결과가 없어도
    다시 보내지 않는다(전송 여부를 모른다). 끝난 스냅샷은 호출 없이 재사용한다. 남은 허용량이 스냅샷과 시험의 보수적 상한을 덮지 못하면 시작하지 않고 멈춘다."""
    caps = caps or preserve.CAPS
    fixs = preserve.fixtures()
    for folder in (RAW, LIVE):
        if folder.is_symlink():
            raise RuntimeError("experiment folder must not be a symlink: " + str(folder))
        folder.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chmod(folder, 0o700)
    led = ledger.Ledger(LEDGER, caps)
    ordered = sorted(runnable, key=lambda c: (c["family_id"], c["source"], c["slot"]))
    tags = {c["cell_id"]: f"t{i}" for i, c in enumerate(sorted(runnable, key=lambda c: c["cell_id"]))}
    snapshots: dict = {}
    try:
        for cell in ordered:
            fix, cell_id = fixs[cell["family_id"]], cell["cell_id"]
            if (RAW / f"{cell_id}.json").exists():
                continue
            if led.attempted(cell_id):
                print("skip, already attempted and never resent:", cell_id, led.status(cell_id), flush=True)
                continue
            label = snapshot_label(cell, fix)
            snap_id = "snap-" + label
            snap = snapshots.get(snap_id)
            need = trial_reserve(cell, fix)
            if snap is None and not led.attempted(snap_id):
                snap_need = snapshot_reserve(fix)
                need = {f: need[f] + snap_need[f] for f in ledger.FIELDS}
            left = led.remaining()
            if any(need[f] > left[f] for f in ledger.FIELDS):
                print(f"stop before {cell_id}: bounded need {need} exceeds the remaining sealed allowance {left}", flush=True)
                break
            if snap is None:
                done = led.ends.get(snap_id)
                if led.attempted(snap_id):
                    if not done or done["status"] != "done" or not (LIVE / f"s{label}").is_dir():
                        print("skip, source attempt never resent:", cell_id, led.status(snap_id), flush=True)
                        continue
                    snap = dict(chat=done["chat"], label=label,
                                source_packet_max_id=done["source_packet_max_id"])
                else:
                    led.start(snap_id, "source", snapshot_reserve(fix), label=label)
                    try:
                        snap = family.build_snapshot(cell["source"], fix, LIVE, label)
                    except family.ReplayError as error:
                        led.end(snap_id, "replay_failed", reason=str(error))
                        print("replay failed, cells skipped:", snap_id, error, flush=True)
                        continue
                    except Exception as error:
                        led.end(snap_id, "error", reason=type(error).__name__)
                        raise
                    led.end(snap_id, "done", snap.get("calls"), chat=snap["chat"],
                            source_packet_max_id=snap["source_packet_max_id"])
                snapshots[snap_id] = snap
            overrides = [["context.evidence.lookup", "true"], [preserve.SAFETY_KEY, online_safety(cell["target"])]]
            if cell["arm"] == "J":
                overrides.append(list(preserve.PACKET_OVERRIDE))
            led.start(cell_id, "trial", trial_reserve(cell, fix), source_snapshot=snap_id, home=tags[cell_id])
            try:
                record = family.run_trial(cell, fix, snap, overrides, LIVE, tags[cell_id])
            except Exception as error:
                led.end(cell_id, "error", reason=type(error).__name__)
                raise
            write_raw(RAW / f"{cell_id}.json", record)
            led.end(cell_id, record["status"], record.get("calls"))
            problems = preserve.capture_report(record)
            print("collected", cell_id, record["status"], "capture problems:", problems, flush=True)
            if problems:
                print("stop: the packet capture does not match the store; fix the engine hook before spending more calls")
                raise SystemExit(1)
    finally:
        print("ledger:", json.dumps(led.report(), sort_keys=True), flush=True)
        led.close()


def denominator(cells: list) -> dict:
    """96칸 분모를 상태별로 센다. 원자료가 있으면 collected, 시작만 있고 결과가 없으면 attempted_unfinished로 남긴다."""
    led = ledger.Ledger(LEDGER, preserve.CAPS)
    try:
        out: dict = {}
        for c in cells:
            if c["status"] != "planned":
                key = "unsupported:" + c["reason"]
            elif (RAW / f"{c['cell_id']}.json").exists():
                key = "collected"
            elif led.attempted(c["cell_id"]):
                key = led.status(c["cell_id"]) if led.status(c["cell_id"]) == "attempted_unfinished" else "attempt_" + led.status(c["cell_id"])
            else:
                fix = preserve.fixtures()[c["family_id"]]
                src = led.status("snap-" + snapshot_label(c, fix))
                key = "not_started" if src in (None, "done") else "source_" + src
            out[key] = out.get(key, 0) + 1
        return dict(sorted(out.items()))
    finally:
        led.close()


def online_safety(target: str) -> str:
    return family.online.SAFETY[target]


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
    if LEDGER.exists():
        print("denominator:", json.dumps(denominator(cells), sort_keys=True))


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
    def check(t=None, r=None, c=None):
        def validate():
            preserve.check_dataset(c or cells, t if t is not None else trials, r or raws, fixs, prices, require_capture=True)

        return validate
    check()()
    # 복원한 source DB의 과거 패킷은 이번 trial 캡처 대상이 아니다.
    sample = next(c for c in cells if c["arm"] == "R" and c["source"] != c["target"])
    sample_record = stub.engine_record(sample, fixs[sample["family_id"]], 0)
    old_packet = dict(sample_record["db"]["packets"][0], id=0,
                      created_at=int(sample_record["t0"] * 1000) - 1000)
    sample_record["db"]["packets"].insert(0, old_packet)
    sample_record["db"]["runs"][0]["started_at"] = int(sample_record["t0"] * 1000) + 1
    assert preserve.capture_report(sample_record) == [], "source packet or run start was mistaken for an outbound capture"
    assert len(cells) == 96 and all(c["status"] == "planned" for c in cells)
    assert any(t["status"] == "delivery_unknown" for t in trials) and any(t["preserve"]["cost_usd"] is None for t in trials)
    def some(arm: str) -> str:
        return next(c["cell_id"] for c in cells if c["arm"] == arm and c["source"] != c["target"])

    def name(cid: str) -> str:
        return f"raw/{cid}.json"

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
        record["captures"].append(dict(record["captures"][0], packet_id=2, attempt=2))

    def capture_edit(**change):
        def apply(record: dict) -> None:
            record["captures"][0].update(change)
        return apply

    def capture_reversed(record: dict) -> None:
        resend(record)
        record["captures"][0]["captured_at_unix_us"] = str(int(record["captures"][1]["captured_at_unix_us"]) + 1000)

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
    semantic("capture missing", r, lambda rec: rec.update(captures=[]))
    semantic("capture field absent", r, lambda rec: rec.pop("captures"))
    semantic("capture body edited after hashing", r, lambda rec: rec["captures"][0].update(body=rec["captures"][0]["body"] + " "))
    semantic("capture hash differs from the store", r, capture_edit(body_hash="0" * 64))
    semantic("capture from another session", r, capture_edit(session=99))
    semantic("capture written before its store row", r, capture_edit(captured_at_unix_us="1000"))
    semantic("capture byte count differs from the body", r, capture_edit(body_bytes=0))
    semantic("captures out of order", r, capture_reversed)
    semantic("extra capture without a store row", r, lambda rec: rec["captures"].append(dict(rec["captures"][0], packet_id=9)))
    semantic("sent body is not the captured body", r, lambda rec: rec.update(sent_body=rec["sent_body"] + "x"))
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

    def null_cost(t: dict) -> bool:
        return t["preserve"]["cost_usd"] is None

    def not_applied(t: dict) -> bool:
        return not t["preserve"]["selector"]["applied"]
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
    def note(event: dict) -> dict:
        return dict(method="TaskEvent", params=dict(event=event))

    replay = [note(dict(Text=dict(text="packet ack", subagent=None))), note(dict(PacketReply=dict())), note(dict(Text=dict(text="42 ", subagent=None))),
              note(dict(Text=dict(text="child", subagent=7))), note(dict(Text=dict(text="done", subagent=None)))]
    assert family.reply_text(replay) == "42 done", "reply text drops the packet reply and subagent text"
    print("simulated dataset: 96 cells, injected defects rejected")


class Crash(BaseException):
    """프로세스가 죽은 상황을 흉내 낸다. Exception이 아니므로 수집기의 오류 처리가 끝 기록을 남기지 못한다."""


def trial_state_checks() -> None:
    """미완료 작업 뒤에는 입력을 더 보내지 않고 시험 홈을 남긴다."""
    class FakeEngine:
        def __init__(self, binary, home, log, env=None):
            self.sock_path = home / "engine.sock"

        def stop(self):
            pass

    for state, status, sent, retained in (
        ("NeedsCheck", "delivery_unknown", 1, True),
        ("Failed", "incomplete", 1, True),
        ("Done", "ok", 3, False),
    ):
        class FakeClient:
            calls: list[str] = []

            def __init__(self, sock_path):
                self.decisions = []

            def call(self, method, params):
                return {}

            def set_model(self, chat, provider, model):
                pass

            def run_input(self, chat, text, allow, timeout):
                self.calls.append(text)
                return dict(status="ok", tasks={1: state}, notes=[])

        with tempfile.TemporaryDirectory(prefix="arm-trial-state-") as place:
            run = Path(place)
            (run / "sa").mkdir()
            (run / "wa").mkdir()
            cell = dict(cell_id="test", source="claude", target="codex", arm="R")
            fix = dict(visible=dict(followups=["one", "two", "three"]))
            snap = dict(label="a", chat=1, source_packet_max_id=0)
            db = dict(runs=[], packets=[], judgments=[])
            with patch.object(family, "Engine", FakeEngine), patch.object(family, "Client", FakeClient), patch.object(family.online, "dump_db", return_value=db):
                record = family.run_trial(cell, fix, snap, [], run)
            assert record["status"] == status and len(FakeClient.calls) == sent, state
            assert (run / "t").exists() == retained, state
    print("trial state checks passed (fake engine and client, no provider)")


def resume_checks() -> None:
    """가짜 호출 계수기로 장부를 시험한다. provider와 engine을 부르지 않고, 코드 경로만 확인한다."""
    import tempfile

    cumulative_only = dict(runs=[dict(input_id=7, started_at=1100)],
                           packets=[dict(created_at=1200)], judgments=[],
                           usage=[dict(scope="ThreadCumulative", at=1300)])
    assert family.call_counts(cumulative_only, 1000) == dict(inputs=2, jev=0)
    assert family.call_counts(dict(runs=[], packets=[], judgments=None), 1000) == dict(inputs=0, jev=None)
    with tempfile.TemporaryDirectory(prefix="arm-overrun-") as place:
        boundary = ledger.Ledger(Path(place) / "ledger", dict(provider_inputs=2, jev_calls=2))
        try:
            boundary.start("x", "trial", dict(inputs=1, jev=1))
            try:
                boundary.end("x", "done", dict(inputs=2, jev=0))
                raise AssertionError("an actual call overrun was accepted")
            except ledger.BudgetStop:
                pass
            assert boundary.committed()["inputs"] == 2
            try:
                boundary.start("y", "trial", dict(inputs=1, jev=0))
                raise AssertionError("the next trial started after an overrun")
            except ledger.BudgetStop:
                pass
        finally:
            boundary.close()

    global RAW, LEDGER, LIVE
    saved = (RAW, LEDGER, LIVE, family.build_snapshot, family.run_trial)
    cells = [c for c in preserve.plan(ALL_CAPS) if c["source"] != c["target"] and c["arm"] in ("R", "J")]
    fixs = preserve.fixtures()
    big = dict(provider_inputs=10 ** 6, jev_calls=10 ** 6)
    order = sorted(cells, key=lambda c: (c["family_id"], c["source"], c["slot"]))

    class Counter:
        def __init__(self, crash_trial=None, crash_snapshot=None, unknown=False):
            self.snapshots, self.trials, self.crash_trial, self.crash_snapshot, self.unknown = [], [], crash_trial, crash_snapshot, unknown

        def build_snapshot(self, source, fix, run, label):
            self.snapshots.append(label)
            if label == self.crash_snapshot:
                (run / f"s{label}").mkdir(parents=True, exist_ok=True)
                raise Crash()
            (run / f"s{label}").mkdir(parents=True)
            n = len(fix["visible"]["users"])
            return dict(chat=7, label=label, source_packet_max_id=0,
                        calls=dict(inputs=None, jev=None) if self.unknown else dict(inputs=n, jev=n))

        def run_trial(self, cell, fix, snap, overrides, run, tag="t"):
            self.trials.append(cell["cell_id"])
            if cell["cell_id"] == self.crash_trial:
                (run / tag).mkdir(parents=True)
                raise Crash()
            record = stub.engine_record(cell, fix, len(self.trials))
            record["calls"] = dict(inputs=None, jev=None) if self.unknown else dict(inputs=3, jev=3)
            return record

    def run(counter: Counter, caps: dict = big) -> None:
        family.build_snapshot, family.run_trial = counter.build_snapshot, counter.run_trial
        with contextlib.redirect_stdout(io.StringIO()):
            run_cells(cells, caps)

    def tags_of(cell_id: str) -> str:
        return next(r["home"] for r in ledger_rows() if r["op"] == "start" and r["id"] == cell_id)

    def ledger_rows() -> list:
        return [json.loads(line) for line in (LEDGER / "ledger.jsonl").read_text().splitlines()]

    try:
        for name in ("crash in a trial after its marker", "crash in a source snapshot", "cap survives restart", "unknown counts are not zero"):
            scratch = tempfile.TemporaryDirectory(prefix="arm-ledger-")
            root = Path(scratch.name)
            RAW, LEDGER, LIVE = root / "raw", root / "ledger", root / "l"
            LIVE.mkdir()
            if name == "crash in a trial after its marker":
                victim = order[0]["cell_id"]
                try:
                    run(Counter(crash_trial=victim))
                    raise AssertionError("the injected crash did not happen")
                except Crash:
                    pass
                assert not (RAW / f"{victim}.json").exists()
                assert any(r["op"] == "start" and r["id"] == victim for r in ledger_rows()) and not any(r["op"] == "end" and r["id"] == victim for r in ledger_rows())
                resumed = Counter()
                run(resumed)
                assert victim not in resumed.trials, "a started cell was sent again"
                assert resumed.snapshots.count(snapshot_label(order[0], fixs[order[0]["family_id"]])) == 0, "a finished snapshot was rebuilt"
                state = denominator(preserve.plan(ALL_CAPS))
                assert state["attempted_unfinished"] == 1 and state["collected"] == len(cells) - 1 and sum(state.values()) == 96, state
                before = len(resumed.trials)
                again = Counter()
                run(again)
                assert not again.trials and not again.snapshots, "a second resume called a provider"
                assert before == len(cells) - 1
                assert (LEDGER / "ledger.jsonl").stat().st_mode & 0o777 == 0o600 and LEDGER.stat().st_mode & 0o777 == 0o700
                assert (LIVE / tags_of(victim)).is_dir(), "the crashed attempt home was removed"
            elif name == "crash in a source snapshot":
                label = snapshot_label(order[0], fixs[order[0]["family_id"]])
                try:
                    run(Counter(crash_snapshot=label))
                    raise AssertionError("the injected crash did not happen")
                except Crash:
                    pass
                resumed = Counter()
                run(resumed)
                assert label not in resumed.snapshots, "a started source snapshot was sent again"
                state = denominator(preserve.plan(ALL_CAPS))
                assert state.get("source_attempted_unfinished", 0) >= 1 and sum(state.values()) == 96, state
                assert (LIVE / f"s{label}").is_dir(), "the crashed snapshot home was removed"
            elif name == "cap survives restart":
                fix = fixs[order[0]["family_id"]]
                cap = dict(provider_inputs=snapshot_reserve(fix)["inputs"] + trial_reserve(order[0], fix)["inputs"], jev_calls=10 ** 6)
                first = Counter()
                run(first, cap)
                assert len(first.trials) == 1, "the cap allowed more than one bounded trial"
                led = ledger.Ledger(LEDGER, cap)
                assert led.committed()["inputs"] <= cap["provider_inputs"] and led.report()["inputs"]["reserved"] <= cap["provider_inputs"]
                led.close()
                second = Counter()
                run(second, cap)
                assert not second.trials and not second.snapshots, "the cap was reset by the restart"
                tight = dict(provider_inputs=1, jev_calls=1)
                none = Counter()
                RAW, LEDGER = root / "raw2", root / "ledger2"
                run(none, tight)
                assert not none.trials and not none.snapshots, "a call started although its bound did not fit"
            else:
                counter = Counter(unknown=True)
                run(counter)
                led = ledger.Ledger(LEDGER, big)
                report = led.report()
                assert report["inputs"]["actual_known"] == 0 and report["inputs"]["actual_unknown_attempts"] == report["attempts"]
                assert report["committed"]["inputs"] == report["inputs"]["reserved"] > 0, "unknown counts were costed as zero"
                led.close()
            scratch.cleanup()
    finally:
        RAW, LEDGER, LIVE, family.build_snapshot, family.run_trial = saved

    # 재생 사전 조건: 고정 입력이 로그 읽기를 요청하고, 로그 본문은 그 한 줄로 확인할 수 있어야 한다. 요청이 없으면 수집 전에 막는다.
    assert family.LOG_FILE == "build.log"
    for fid, fix in fixs.items():
        v = fix["visible"]
        assert family.log_read_requested(fix), "fixture never asks to read the log: " + fid
        assert v["tool_results"][0].count("build log for") == 1 and v["users"] and len(v["followups"]) == 3
        assert fix["fixture_hash"] == preserve.sha(preserve.canonical(v)), "fixture hash is stale"
    same = next(f for f in fixs.values() if f["visible"]["kind"] == "same-string-inputs")["visible"]["users"]
    assert same[0] == same[2], "the repeated input must stay identical"
    silent = copy.deepcopy(next(iter(fixs.values())))
    silent["visible"]["users"] = ["hello"]
    assert log_read_blockers(cells[:1], {cells[0]["family_id"]: silent}), "a family without a log read request was not blocked"
    for handler in (lambda: family.build_snapshot("claude", silent, Path("/nonexistent-arm"), "x"),):
        must_fail("snapshot for a fixture without a log read request", handler)
    print("resume, cap and replay precondition checks passed (fake call counter, no provider)")


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
    trial_state_checks()
    resume_checks()
    cells = preserve.plan(preserve.default_capabilities())
    reasons = {r: [c["reason"] for c in cells].count(r) for r in sorted({c["reason"] for c in cells if c["reason"]})}
    print("current plan:", json.dumps(reasons, sort_keys=True), "executable", sum(c["status"] == "planned" for c in cells), "of", len(cells))
    if LEDGER.exists():
        print("denominator:", json.dumps(denominator(cells), sort_keys=True))
    if not any(RAW.glob("*.json")) if RAW.exists() else True:
        print("no collected data: 0 of", len(cells), "cells measured")
        return
    manifest = load_manifest()
    cells, trials, fixs = build_all(manifest, load_raw())
    preserve.check_dataset(cells, trials, load_raw(), fixs, manifest["prices"], require_capture=True)
    check_repeat(summary_bytes, cells, trials)
    print("collected data verified:", len(trials))


if __name__ == "__main__":
    {"plan": lambda: print(json.dumps(preserve.analyze(preserve.plan(preserve.default_capabilities()), []), indent=2, sort_keys=True)),
     "seal": seal, "collect": collect, "process": process, "verify": verify}[sys.argv[1]]()
