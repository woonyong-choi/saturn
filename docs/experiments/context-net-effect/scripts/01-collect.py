"""수집. `01-collect.py <provider> formal|pilot [--workers N]`.

실제 engine과 provider를 부른다. 설계와 이 실행기를 커밋한 뒤에만 formal을 시작한다.
저장소와 홈은 worktree의 `.runtime/context-net-effect/<run>/` 아래에 두고 커밋하지 않는다.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import gzip
import json
import shutil
import sqlite3
import subprocess
import sys
import threading
import time
from pathlib import Path

import engine_driver
import fixtures
import plan

HERE = Path(__file__).resolve().parent
EXP = HERE.parent
REPO = EXP.parents[2]
BIN = REPO / "target" / "release"
# 소켓 경로 한도(SUN_LEN) 때문에 짧은 이름을 쓴다: <run 첫 글자>/<provider 태그 a|x><시드><조건 첫 글자>
TAG = {"claude": "a", "codex": "x", plan.CROSS: "k", plan.STAGE2: "y"}
RUNTIME = REPO / ".runtime" / "cne"
DATA = EXP / "data" / "raw"

TABLES = ["chats", "inputs", "sessions", "runs_meta", "usage", "judgments", "handoff_packets", "handoff_packet_items", "evidence_lookups", "events"]

class StartupUnsent(RuntimeError):
    pass


lock = threading.Lock()
used = {"claude": 0, "codex": 0, plan.CROSS: 0, plan.STAGE2: 0}


def extract(db: Path) -> dict:
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    con.row_factory = sqlite3.Row
    out = {}
    for table in TABLES:
        if table == "events":
            rows = con.execute("SELECT chat_id,seq,run_id,body,at FROM events ORDER BY chat_id,seq")
        elif table == "runs_meta":
            rows = con.execute("SELECT id,chat_id,input_id,task_id,agent_id,session_id,provider,effect_scope,started_at,ended_at,end_kind FROM runs ORDER BY id")
        else:
            rows = con.execute(f"SELECT * FROM {table} ORDER BY 1")
        out[table] = [dict(r) for r in rows]
    con.close()
    return out


STARTUP_RETRIES = 4


def startup_unsent(base: Path, rec: dict) -> bool:
    """첫 입력이 접수되기 전에 router 점검에서 멈췄고 기록 저장소에도 입력이 없으면, 보내지 않았음이 확정된 실패다."""
    if rec["status"] != "failed" or "router key required" not in rec["stdout"] or "> [A]" in rec["stdout"]:
        return False
    db = base / "h" / "saturn.db"
    if not db.exists():
        return True
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    try:
        return con.execute("SELECT count(*) FROM inputs").fetchone()[0] == 0
    finally:
        con.close()


def run_trial(run: str, provider: str, seed: int, arm: str, key: str) -> dict:
    """보내지 않았음이 확정된 시작 실패만 새 저장소로 다시 시작한다(최대 STARTUP_RETRIES회). 실패 기록은 결과에 남긴다."""
    failures: list[str] = []
    for attempt in range(STARTUP_RETRIES + 1):
        result = run_trial_once(run, provider, seed, arm, key, failures)
        if result is not None:
            result["startup_failures"] = failures
            return result
        time.sleep(30 * (attempt + 1))
    raise RuntimeError("startup failed repeatedly before any input was accepted")


def run_trial_once(run: str, provider: str, seed: int, arm: str, key: str, failures: list[str]) -> dict | None:
    trial = f"{provider}-{seed}-{arm}"
    base = RUNTIME / run[0] / f"{TAG[provider]}{seed}{arm[0]}"
    result_file = base / "trial.json"
    if result_file.exists():
        return json.loads(result_file.read_text())
    if base.exists():
        raise RuntimeError(f"partial trial directory, not retrying automatically: {base}")
    facts = fixtures.build(base, seed)
    chat = engine_driver.Chat(BIN, base / "h", base / "practice", key)
    turns: list[dict] = []
    stop = False

    def say(label: str, text: str, overrides: list[str]) -> bool:
        with lock:
            if used[provider] >= plan.CALL_CAPS[provider]:
                raise RuntimeError("provider request cap reached")
            used[provider] += 1
        rec = chat.send(text, overrides)
        rec["label"] = label
        if label == "s1" and startup_unsent(base, rec):
            raise StartupUnsent(rec["stdout"][:200])
        turns.append(rec)
        return rec["status"] == "ok"

    cross = provider == plan.CROSS
    try:
        return _run_body(run, provider, seed, arm, base, facts, chat, turns, say, cross, result_file)
    except StartupUnsent as err:
        chat.stop_engine()
        failures.append(str(err))
        shutil.rmtree(base)
        with lock:
            used[provider] -= 1  # 보내지 않았으므로 요청 수에 넣지 않는다
        return None


def _run_body(run, provider, seed, arm, base, facts, chat, turns, say, cross, result_file):
    trial = f"{provider}-{seed}-{arm}"
    base_ov = plan.base_overrides(provider, arm)
    if cross:
        (base / "h" / "config.toml").write_text('[model]\nmode = "manual"\ndefault = "claude/haiku"\n')  # 첫 고르기 창을 건너뛴다
    ok = True
    for i, text in enumerate(plan.SETUP_TURNS, 1):
        ok = say(f"s{i}", text.format(**facts), base_ov)
        if not ok:
            break
    grades: dict = {}
    if ok:
        ok = say("boundary", plan.BOUNDARY_TURN, base_ov + ([] if cross else plan.boundary_overrides(provider, arm)))
    if ok and arm == "provider":
        ok = say("compact", "/compact", base_ov)
    follow = plan.FOLLOW_UPS
    send = None
    tui = None
    if ok and cross:
        tui = engine_driver.TuiSwitch(chat, f"cne{seed}{arm[0]}", [*plan.COMMON_OVERRIDES, *plan.cross_overrides(arm)])
        tui.start()
        screens = tui.pick_model("codex", plan.CROSS_MODEL_LABEL)
        picked = "gpt-5.6-luna" in screens.lower().rsplit("----", 1)[-1]
        turns.append({"label": "switch", "status": "ok" if picked else "failed", "stdout": screens, "text": "/model codex", "overrides": tui.overrides, "started_unix": 0, "ended_unix": 0, "exit_code": None})
        ok = picked

        def send(label: str, text: str) -> bool:
            with lock:
                if used[provider] >= plan.CALL_CAPS[provider]:
                    raise RuntimeError("provider request cap reached")
                used[provider] += 1
            rec = tui.send(text)
            rec["label"] = label
            turns.append(rec)
            return rec["status"] == "ok"
    if ok:
        for label, text in follow:
            sent_ok = send(label, text.format(**facts)) if send else say(label, text.format(**facts), base_ov)
            if not sent_ok:
                ok = False
                break
            if label in ("f1", "f3"):
                grades[label] = fixtures.grade(base)
    if tui:
        tui.close()
    stopped = chat.stop_engine()
    data = extract(base / "h" / "saturn.db")
    result = {
        "trial": trial,
        "provider": provider,
        "seed": seed,
        "arm": arm,
        "run": run,
        "plan_version": plan.PLAN_VERSION,
        "facts": facts,
        "complete": ok,
        "turns": turns,
        "grades": grades,
        "engine_pids_stopped": stopped,
        "store": data,
    }
    result_file.write_text(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return result


def publish(result: dict) -> None:
    """원자료 한 건을 gzip으로 data/raw에 둔다. 키는 어디에도 들어 있지 않다."""
    DATA.mkdir(parents=True, exist_ok=True)
    raw = json.dumps(result, ensure_ascii=False, sort_keys=True).encode()
    with gzip.GzipFile(DATA / f"{result['run']}-{result['trial']}.json.gz", "wb", mtime=0) as f:
        f.write(raw)


def write_env() -> None:
    """실행 환경을 env.json에 한 번 쓴다. 이미 있으면 덮어쓰지 않는다."""
    path = EXP / "env.json"
    if path.exists():
        return

    def out(*cmd: str) -> str:
        return subprocess.run(cmd, capture_output=True, text=True, cwd=REPO).stdout.strip()

    path.write_text(json.dumps({
        "os": out("uname", "-srm"),
        "python": sys.version.split()[0],
        "claude_code": out("claude", "--version"),
        "codex_cli": out("codex", "--version"),
        "saturn_commit": out("git", "rev-parse", "HEAD"),
        "router_model": "jev-1.13.0",
        "models": {k: v["model"] for k, v in plan.PROVIDERS.items()},
        "t_abs": {k: v["t_abs"] for k, v in plan.PROVIDERS.items()},
        "seeds": {"formal": list(plan.FORMAL_SEEDS), "cross": list(plan.CROSS_SEEDS), "order": plan.ORDER_SEED, "bootstrap": 7007},
        "plan_version": plan.PLAN_VERSION,
    }, indent=1, sort_keys=True) + "\n")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("provider", choices=sorted(plan.PROVIDERS))
    ap.add_argument("mode", choices=["formal", "pilot"])
    ap.add_argument("--workers", type=int, default=3)
    ap.add_argument("--only", help="seed:arm 하나만")
    args = ap.parse_args()
    seeds = (plan.CROSS_SEEDS if args.provider == plan.CROSS else plan.STAGE2_SEEDS if args.provider == plan.STAGE2 else plan.FORMAL_SEEDS) if args.mode == "formal" else plan.PILOT_SEEDS
    run = args.mode
    pairs = plan.trial_list(args.provider, seeds)
    if args.only:
        s, a = args.only.split(":")
        pairs = [(int(s), a)]
    write_env()
    key = engine_driver.router_key()
    done = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = {}
        for index, (s, a) in enumerate(pairs):
            if 0 < index < args.workers:
                time.sleep(15)  # engine 시작이 한꺼번에 몰리면 router 점검이 실패한다(예비 실행에서 관측)
            futures[pool.submit(run_trial, run, args.provider, s, a, key)] = (s, a)
        for fut in concurrent.futures.as_completed(futures):
            s, a = futures[fut]
            try:
                res = fut.result()
                publish(res)
                done += 1
                print(f"{args.provider} {s} {a} complete={res['complete']} ({done}/{len(pairs)})", flush=True)
            except Exception as exc:  # 한 trial의 오류가 다른 trial을 멈추지 않는다. 자동 재시도하지 않는다.
                print(f"{args.provider} {s} {a} ERROR {type(exc).__name__}: {exc}", flush=True)
    print("provider requests used", used, flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
