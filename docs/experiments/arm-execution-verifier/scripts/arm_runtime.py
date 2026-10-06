"""기존 실험의 호출 경계와 집계 함수를 재사용하고 이번 실행의 원자료를 분리한다."""

from __future__ import annotations

import importlib.util
import os
import sys
from pathlib import Path
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
JEV_SCRIPTS = PUBLIC.parent / "jev-role-efficiency/scripts"
CALLER = os.environ.get("ARM_VERIFIER_CALLER", "stub")
if CALLER not in ("stub", "live"):
    raise SystemExit("ARM_VERIFIER_CALLER must be stub or live")
PRIVATE = ROOT / ".local/experiments/arm-execution-verifier" / CALLER
RESULTS = PUBLIC / "results" if CALLER == "live" else PRIVATE / "results"
SEED = 540001
POLICY_VERSION = "arm-verifier-1"
LIMITS = {"claude": 260, "codex": 0, "jev": 60}

sys.path.insert(0, str(JEV_SCRIPTS))
import runtime as jev  # noqa: E402  jev-role-efficiency의 호출 경계

jev.PRIVATE = PRIVATE
jev.BASE.PRIVATE = PRIVATE
jev.BASE.LIMITS = LIMITS
read, write, rows, digest = jev.read, jev.write, jev.rows, jev.digest


def load_script(path: Path, name: str) -> Any:
    """숫자로 시작하는 기존 스크립트 파일을 모듈로 불러온다."""
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


collect = load_script(JEV_SCRIPTS / "01-collect.py", "jev_role_collect")
analysis = load_script(JEV_SCRIPTS / "02-analyze.py", "jev_role_analyze")
