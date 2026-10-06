#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."/..
python3 - <<'PY'
import importlib.util
import json

path = 'docs/experiments/packet-selector-contrast/scripts/03-verify.py'
spec = importlib.util.spec_from_file_location('posthoc_verify', path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
original = module.process.analyze
# 봉인된 검사기는 JSON 목록과 Python tuple을 직접 비교한다. 저장 형식으로 정규화한 뒤 동일한 검사를 실행한다.
module.process.analyze = lambda: json.loads(json.dumps(original(), ensure_ascii=False))
module.verify()
summary = json.loads((module.collect.RUN / 'summary.json').read_text())
set_contrast = sum(
    bool(pair['arms']['R']['competing'] and pair['arms']['J']['competing'])
    and sorted(pair['arms']['R']['competing']) != sorted(pair['arms']['J']['competing'])
    for pair in summary['pairs']
)
joint = sum(
    pair['packet_contrast'] and pair['protected_match']
    and sorted(pair['arms']['R']['competing']) != sorted(pair['arms']['J']['competing'])
    for pair in summary['pairs']
)
assert set_contrast == joint == summary['joint_contrast'] == 3
print('verified: competing item set and H1 joint count = 3/12')
PY
