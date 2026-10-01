"""수집과 처리가 같이 쓰는 경로와 설정."""
import json
import os
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
EXP = os.path.dirname(HERE)
REPO = os.path.abspath(os.path.join(EXP, "..", "..", ".."))
ORCH = os.path.expanduser("~/workspace/woon/.local/orchestration/saturn-experiments/record-fidelity")
EXAMPLE = os.path.join(REPO, "target", "debug", "examples", "record-fidelity")
SHIM = os.path.join(HERE, "shim.py")
PROVIDERS = ["codex", "claude"]
MODELS = {"codex": "gpt-6-sol", "claude": "claude-opus-5-5"}
# 승인과 설정 출처는 provider 인자로 정한다. Saturn 기본값(작업 폴더 쓰기 허용)과 같은 범위다.
EXTRA = {
    "codex": ["-c", 'approval_policy="never"', "-c", 'sandbox_mode="workspace-write"'],
    "claude": ["--dangerously-skip-permissions", "--setting-sources", "project"],
}


def params():
    with open(os.path.join(EXP, "env.json"), encoding="utf-8") as handle:
        return json.load(handle)["params"]


def build_example():
    subprocess.run(
        ["cargo", "build", "-q", "-p", "saturn-engine", "--example", "record-fidelity"],
        cwd=REPO, check=True,
    )
