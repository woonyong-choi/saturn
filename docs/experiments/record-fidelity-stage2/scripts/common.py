"""수집과 처리가 같이 쓰는 경로와 설정."""
import json
import os
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
EXP = os.path.dirname(HERE)
REPO = os.path.abspath(os.path.join(EXP, "..", "..", ".."))
SOURCE_RAW = os.path.join(EXP, "..", "record-fidelity", "data", "raw")
ORCH = os.path.expanduser("~/workspace/woon/.local/orchestration/saturn-experiments/record-fidelity-stage2")
EXAMPLE = os.path.join(REPO, "target", "debug", "examples", "record-fidelity")
PACKET = os.path.join(REPO, "target", "debug", "examples", "packet")
SHIM = os.path.join(HERE, "shim.py")
PROVIDERS = ["codex", "claude"]
MODELS = {"codex": "gpt-6-sol", "claude": "claude-opus-5-5"}


def params():
    with open(os.path.join(EXP, "env.json"), encoding="utf-8") as handle:
        return json.load(handle)["params"]


def build_examples():
    subprocess.run(
        ["cargo", "build", "-q", "-p", "saturn-engine", "--example", "record-fidelity",
         "-p", "saturn-core", "--example", "packet"],
        cwd=REPO, check=True,
    )
