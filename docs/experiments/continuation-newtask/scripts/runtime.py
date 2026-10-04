"""봉인된 앞 실험의 질문·라벨러를 불러오고 이번 저장 경로와 예산을 적용한다."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
WORKTREE = PUBLIC.parents[2]
PRIVATE = WORKTREE / ".local/experiments/continuation-newtask"
BASE = Path.home() / "workspace/oss/saturn/.local/experiments/continuation-ko"
SEED = 608
MODELS = ("gpt-6-astra", "gpt-5.6-luna")


def load(name: str, path: Path) -> Any:
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


prior_dir = PUBLIC.parent / "continuation-misjoin/scripts"
support = load("support", prior_dir / "support.py")
support.PRIVATE = PRIVATE
support.LIMITS = {"jev": 4000, "codex": 500}
collector = load("misjoin_collector", prior_dir / "01-collect.py")
analysis = load("misjoin_analysis", prior_dir / "02-analyze.py")
labeler = support.load("newtask_labeler", "02-label.py")
labeler.PRIVATE = PRIVATE
labeler.reserve = support.reserve
labeler.setup = support.setup
extractor = support.load("newtask_claude", "01-collect.py")
metrics = analysis.metrics
read, write, rows = support.read, support.write, support.rows
append, reserve, setup = support.append, support.reserve, support.setup
sha, now = support.sha, support.now
mask = labeler.mask
