"""사전 지정한 단측 이항 검정력의 최소 표본 크기를 다시 계산한다."""

from __future__ import annotations

import json

from runtime import analysis


def calculate() -> dict:
    for n in range(1, 3001):
        critical = -1
        for k in range(n + 1):
            if analysis.cdf(k, n, 0.05) > 0.0125:
                break
            critical = k
        power = analysis.cdf(critical, n, 0.02)
        if power >= 0.80:
            return {
                "alpha": 0.0125,
                "null_rate": 0.05,
                "alternative_rate": 0.02,
                "required_new": n,
                "critical_k": critical,
                "power": power,
                "type_one_error": analysis.cdf(critical, n, 0.05),
            }
    raise RuntimeError("power target outside search bound")


if __name__ == "__main__":
    print(json.dumps(calculate()))
