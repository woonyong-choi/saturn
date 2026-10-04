"""번호가 있는 Jev 스크립트의 합의 라벨 함수를 가져온다."""

import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("jev_module", Path(__file__).with_name("03-jev.py"))
jev_module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(jev_module)
load_consensus = jev_module.load_consensus
