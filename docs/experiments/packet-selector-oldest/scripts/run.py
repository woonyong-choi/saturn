"""봉인된 패킷 선별 실행기를 재사용하는 oldest 근거 확인 실행기."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import random
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
ROOT = HERE.parents[2]
BASE = HERE.parent / "packet-selector-contrast/scripts"


def module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


collector = module("oldest_collect_base", BASE / "01-collect.py")
processor = module("oldest_process_base", BASE / "02-process.py")

collector.RUN = ROOT / ".runtime/so"
collector.SEED = 54026008
processor.collect = collector


def files(index: int) -> tuple[dict[str, str], int, str]:
    rng = random.Random(collector.SEED + index)
    numbers = rng.sample(range(18, 89), 3)
    names = collector.NAMES
    out = {}
    for i, name in enumerate(names):
        release = "current" if i == 0 else "old"
        out[name] = f"FILE={name} RELEASE={release} TIMEOUT={numbers[i]} seconds\n" + (f"archived detail number {i} for {name}. " * 430)
    return out, numbers[0], names[0]


def hashes() -> dict[str, str]:
    sources = [HERE / "design.md", Path(__file__), BASE / "01-collect.py", BASE / "02-process.py"]
    return {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}


collector.source_files = files
collector.protocol_hashes = hashes


def source(index: int) -> tuple[int | None, bool]:
    label = f"s{index:02}"
    home, work = collector.RUN / label / "home", collector.RUN / label / "work"
    source_files, _, _ = files(index)
    home.mkdir(parents=True)
    work.mkdir(parents=True)
    for name, body in source_files.items():
        (work / name).write_text(body)
    env = dict(os.environ, SATURN_HOME=str(home))
    engine = collector.Engine(collector.BINARY, home, collector.RUN / label / "engine.log", env=env)
    client = collector.Client(engine.sock_path)
    chat = None
    outputs = []
    try:
        chat = client.attach(work, [])
        client.set_model(chat, "codex", "gpt-5.6-luna")
        for name in collector.NAMES:
            collector.journal(f"{label}-{name}")
            prompt = f"Run exactly this shell command and nothing else: `cat {name}`. After reading its output, reply only READY."
            output = client.run_input(chat, prompt, allow=lambda s: ".log" in s, timeout=300)
            outputs.append({"name": name, "status": output["status"], "tasks": output.get("tasks"), "notes": output.get("notes", [])})
            if output["status"] != "ok" or not output.get("tasks") or any(x != "Done" for x in output["tasks"].values()):
                break
    finally:
        client.close()
        engine.stop()
    db = collector.db_rows(home / "saturn.db", chat) if chat else {}
    seen = {name: any(e["body"].startswith('{"ToolResult"') and f"FILE={name}" in e["body"] for e in db.get("events", [])) for name in collector.NAMES}
    valid = len(outputs) == 3 and all(seen.values()) and all(o["status"] == "ok" and all(v == "Done" for v in o["tasks"].values()) for o in outputs)
    collector.save(collector.RUN / "raw" / f"{label}.json", {"chat": chat, "outputs": outputs, "seen": seen, "valid": valid, "db": db})
    return chat, valid


collector.source = source


def analyze() -> None:
    summary = processor.analyze()
    summary["competing_set_contrast"] = sum(
        bool(p["arms"]["R"]["competing"] and p["arms"]["J"]["competing"])
        and sorted(p["arms"]["R"]["competing"]) != sorted(p["arms"]["J"]["competing"])
        for p in summary["pairs"]
    )
    summary["joint_set_contrast"] = sum(
        p["packet_contrast"] and p["protected_match"]
        and sorted(p["arms"]["R"]["competing"]) != sorted(p["arms"]["J"]["competing"])
        for p in summary["pairs"]
    )
    b, c = summary["jev_only_correct"], summary["rrf_only_correct"]
    n = b + c
    summary["mcnemar_p_two_sided"] = min(1.0, 2 * sum(__import__("math").comb(n, k) for k in range(min(b, c) + 1)) / (2**n)) if n else 1.0
    summary["h1"] = summary["joint_set_contrast"] >= 9 and summary["protected_mismatch"] == 0
    summary["h2"] = summary["h1"] and b >= 6 and c == 0 and summary["mcnemar_p_two_sided"] < 0.05
    summary["h3"] = bool(summary["h2"] and summary["complete_pairs"] == 12 and summary["cost_diff_ci95_usd"] and summary["duration_diff_ci95_s"]
                         and summary["cost_diff_ci95_usd"][1] < 0 and summary["duration_diff_ci95_s"][1] < 0)
    collector.save(collector.RUN / "summary.json", summary)
    print(json.dumps({k: v for k, v in summary.items() if k != "pairs"}, ensure_ascii=False, indent=2))


def verify() -> None:
    collector.check_seal()
    saved = json.loads((collector.RUN / "summary.json").read_text())
    assert saved["joint_set_contrast"] <= saved["competing_set_contrast"]
    assert saved["planned_pairs"] == 12
    for pair in saved["pairs"]:
        if not pair["source_valid"]:
            assert not pair["arms"]["R"]["collected"] and not pair["arms"]["J"]["collected"]
        for arm in ("R", "J"):
            trial = pair["arms"][arm]
            if trial["collected"]:
                assert trial["capture_ok"] and trial["selector_applied"]
    print("verified: seal, source denominator, captures and applied selectors")


if __name__ == "__main__":
    os.umask(0o077)
    {"seal": collector.seal, "collect": collector.collect, "analyze": analyze, "verify": verify}[sys.argv[1]]()
