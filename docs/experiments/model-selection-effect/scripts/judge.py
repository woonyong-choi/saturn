"""한 저장소에 계열의 시험 호출을 실행해 결과를 JSON으로 낸다. 에이전트 코드는 이 별도 프로세스 안에서만 불러온다."""

from __future__ import annotations

import importlib
import json
import random
import signal
import sys
from decimal import Decimal
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import families  # noqa: E402


def norm(value):
    if isinstance(value, Decimal):
        return str(value)
    if isinstance(value, dict):
        return {str(k): norm(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [norm(v) for v in value]
    if isinstance(value, float):
        return repr(value)
    if isinstance(value, (str, int, bool)) or value is None:
        return value
    return repr(value)


def main() -> None:
    family, params_json, seed, root = sys.argv[1:5]
    root_path = Path(root).resolve()
    sys.path.insert(0, str(root_path))

    def load(rel: str):
        name = rel[:-3].replace("/", ".")
        for loaded in [n for n in sys.modules if n == name.split(".")[0] or n.startswith(name.split(".")[0] + ".")]:
            if loaded not in sys.modules:
                continue
        return importlib.import_module(name)

    spec = families.FAMILIES[family]["build"](json.loads(params_json))
    cases = spec["cases"](random.Random(int(seed)))
    results = {}

    def on_alarm(signum, frame):
        raise TimeoutError("case timeout")

    signal.signal(signal.SIGALRM, on_alarm)
    for label, call in cases:
        signal.alarm(5)
        try:
            results[label] = norm(call(load, root_path))
        except BaseException as error:  # noqa: BLE001
            results[label] = ["harness-raise", type(error).__name__]
        finally:
            signal.alarm(0)
    print(json.dumps(results, sort_keys=True, ensure_ascii=False))


if __name__ == "__main__":
    main()
