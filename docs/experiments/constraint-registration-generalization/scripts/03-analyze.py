"""원응답에서 처리 결과를 집계하고 정답 없는 정확도는 미평가로 남긴다."""

from __future__ import annotations

import hashlib
import json
import math
import random
import statistics
import sys
from collections import Counter
from pathlib import Path

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/constraint-registration-generalization"


def wilson(k: int, n: int, alpha: float = 0.05) -> list[float] | None:
    if not n:
        return None
    z = statistics.NormalDist().inv_cdf(1 - alpha / 2)
    p = k / n
    center = (p + z * z / (2 * n)) / (1 + z * z / n)
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / (1 + z * z / n)
    return [max(0, center - half), min(1, center + half)]


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    return sorted(values)[max(0, math.ceil(fraction * len(values)) - 1)]


def bootstrap_p95(values: list[float]) -> list[float] | None:
    if not values:
        return None
    rng = random.Random(38220261005)
    points = [
        percentile(rng.choices(values, k=len(values)), 0.95) for _ in range(10000)
    ]
    return [percentile(points, 0.003125), percentile(points, 0.996875)]


def verify() -> None:
    seal = json.loads((PRIVATE / "design-seal.json").read_text())
    for name, expected in seal["hashes"].items():
        if hashlib.sha256((ROOT / name).read_bytes()).hexdigest() != expected:
            raise RuntimeError("seal mismatch: " + name)
    cases = json.loads((PRIVATE / "samples.json").read_text())
    ids = {case["sample_id"] for case in cases}
    if len(ids) != len(cases):
        raise RuntimeError("duplicate sample id")
    ledger = [
        json.loads(line) for line in (PRIVATE / "calls.jsonl").read_text().splitlines()
    ]
    reserved = [row["sample_id"] for row in ledger]
    if len(set(reserved)) != len(reserved) or not set(reserved) <= ids:
        raise RuntimeError("invalid reservations")
    for path in (PRIVATE / "raw").glob("*.json"):
        record = json.loads(path.read_text())
        if record["sample_id"] not in reserved or path.stem != record["sample_id"]:
            raise RuntimeError("unreserved response")
        if "Authorization" in record or "headers" in record:
            raise RuntimeError("credential header in record")
    print(
        json.dumps({"verify": "ok", "samples": len(cases), "reserved": len(reserved)})
    )


# cost: io 원응답 한 번 읽기; basis: estimate
def process() -> list[dict]:
    records = [
        json.loads(path.read_text())
        for path in sorted((PRIVATE / "raw").glob("*.json"))
    ]
    rows = [
        {
            key: record[key]
            for key in ("run_id", "sample_id", "ts_utc", "status", "latency_s")
        }
        | {
            "probabilities": record.get("probabilities"),
            "usage": record.get("response", {}).get("usage"),
            "error_type": record.get("error_type"),
            "http_status": record.get("http_status"),
        }
        for record in records
    ]
    (PRIVATE / "processed.jsonl").write_text(
        "".join(json.dumps(row) + "\n" for row in rows)
    )
    return rows


def main() -> None:
    if len(sys.argv) > 1 and sys.argv[1] == "verify":
        verify()
        return
    rows = process()
    if len(sys.argv) > 1 and sys.argv[1] == "process":
        print(json.dumps({"processed": len(rows)}))
        return
    cases = json.loads((PRIVATE / "samples.json").read_text())
    census = json.loads((PRIVATE / "census.json").read_text())
    valid = [row for row in rows if row["status"] == "ok"]
    latencies = [row["latency_s"] for row in rows]
    attempts = len((PRIVATE / "calls.jsonl").read_text().splitlines())
    incomplete = attempts - len(rows)
    failed = attempts - len(valid)
    usage = sum((row.get("usage") or {}).get("input_tokens", 0) for row in rows)
    arms = []
    for arm, auto, ask in (
        ("baseline", 0.9, 0.7),
        ("korean", 0.77, 0.43),
        ("choice", 0.9, 0.7),
    ):
        arms.append(
            {
                "arm": arm,
                "auto_threshold": auto,
                "ask_threshold": ask,
                "auto_count": sum(row["probabilities"][arm] >= auto for row in valid),
                "ask_count": sum(
                    ask <= row["probabilities"][arm] < auto for row in valid
                ),
                "precision": None,
                "recall": None,
                "reason": "independent labels unavailable",
            }
        )
    failure_ci = wilson(failed, attempts, 0.00625)
    latency_ci = bootstrap_p95(latencies) if not incomplete else None
    h4 = (
        "보류"
        if not failure_ci or not latency_ci
        else (
            "채택"
            if failure_ci[1] <= 0.01 and latency_ci[1] < 1
            else "기각"
            if failure_ci[0] > 0.01 or latency_ci[0] >= 1
            else "보류"
        )
    )
    summary = {
        "samples": len(cases),
        "attempts": attempts,
        "completed": len(rows),
        "valid": len(valid),
        "failed": failed,
        "incomplete": incomplete,
        "status_counts": dict(Counter(row["status"] for row in rows)),
        "error_types": dict(
            Counter(row["error_type"] for row in rows if row.get("error_type"))
        ),
        "labels_available": 0,
        "input_tokens": usage,
        "estimated_usd": usage * 0.042 / 1e6,
        "usage_missing": sum(not row.get("usage") for row in rows),
        "p50_s": statistics.median(latencies) if latencies else None,
        "p95_s": percentile(latencies, 0.95),
        "p95_ci99375": latency_ci,
        "failure_ci99375": failure_ci,
        "arms": arms,
        "hypotheses": {
            "H1": "미평가: 독립 정답 없음",
            "H2": "미평가: 독립 정답 없음",
            "H3": "부분 측정: 확인 비율만 가능",
            "H4": h4,
        },
        "census": census,
        "raw_digest": hashlib.sha256(
            "".join(
                hashlib.sha256(path.read_bytes()).hexdigest()
                for path in sorted((PRIVATE / "raw").glob("*.json"))
            ).encode()
        ).hexdigest(),
    }
    (PUBLIC / "results").mkdir(exist_ok=True)
    (PUBLIC / "results/summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2) + "\n"
    )
    lines = [
        "# 지속 제약 등록의 대화 표본 확대: 실험 결과",
        "",
        "## 요약",
        "",
        f"새 대화 입력 {len(cases):,}건 중 {attempts:,}회 호출했다. 유효 응답은 {len(valid):,}건이고 실패·미완료는 {failed:,}건이다. 독립 정답이 없어 90% 정밀도 달성 여부는 미평가다.",
        "",
        "## 방법",
        "",
        "[실험 설계](design.md)와 questions.json을 호출 전에 해시로 봉인했다. 이전 400건이 속한 세션을 제외했다. 원응답과 호출 예약을 비공개 실험 공간에 보존했다.",
        "",
        "## 설계와 다른 점",
        "",
        "기존 커밋 금지 지시를 유지해 사전 커밋 대신 UTC 시각과 SHA-256 봉인을 사용했다. 새 표본의 독립 정답이 확보되지 않아 H1~H3의 정확도 분석은 수행하지 않았다.",
        "",
        "## 결과",
        "",
        "### 흐름",
        "",
        "| 항목 | 수 |",
        "|---|---|",
        f"| 추출 후보 | {census['eligible']:,} |",
        f"| 선택 | {len(cases):,} |",
        f"| 호출 | {attempts:,} |",
        f"| 유효 | {len(valid):,} |",
        f"| 실패·미완료 | {failed:,} |",
        "",
        "### 처리 분포",
        "",
        "아래 개수는 모델이 선택한 후보 수이며 정답 개수가 아니다.",
        "",
        "| 질문 | 자동 기준 | 자동 후보 | 확인 후보 |",
        "|---|---|---|---|",
    ]
    lines.extend(
        f"| {arm['arm']} | {arm['auto_threshold']} | {arm['auto_count']} | {arm['ask_count']} |"
        for arm in arms
    )
    lines.extend(
        [
            "",
            "### 지연과 비용",
            "",
            f"p95는 {summary['p95_s']:.3f}초이며 보정 bootstrap 구간은 [{latency_ci[0]:.3f}, {latency_ci[1]:.3f}]초다."
            if latency_ci
            else "미완료 요청이 있어 지연 구간은 계산하지 않았다.",
            f"확인된 입력 토큰은 {usage:,}개, [공식 단가](https://docs.typesafe.ai/models)의 예상 비용은 USD {summary['estimated_usd']:.4f}다. usage 없는 요청 {summary['usage_missing']}건의 비용은 포함하지 않았다.",
            "",
            "## 논의",
            "",
            "### 해석",
            "",
            "호출 성공과 판단 정답은 다르다. 새 표본의 자동 후보를 독립 판정하기 전에는 정밀도나 재현율을 제시하지 않는다. 기존 400건에서 탐색한 90.9%를 새 표본의 정확도로 옮기지 않는다.",
            "",
            "### 타당성 위협",
            "",
            "한 사용자의 대화이며 프로젝트와 세션이 독립이지 않다. 역할 필터를 통과한 붙여넣기·자동 입력이 남을 수 있다. 세 질문을 한 요청에 넣었으므로 개별 질문의 지연이나 비용은 분리 측정하지 않았다. 정답 부재와 사후 의도 차이 때문에 모델 품질 채택은 하지 않는다. 서버 상태와 모델 응답은 같은 버전에서도 달라질 수 있다.",
            "",
            "## 재현",
            "",
            "```sh",
            "sh run.sh verify",
            "sh run.sh analyze",
            "```",
            "",
            "재현에는 비공개 표본과 원응답이 필요하다. 분석은 원응답을 바꾸거나 API를 다시 호출하지 않는다.",
            "",
            "## 결론",
            "",
            "| 가설 | 판정 | 반영한 문서 |",
            "|---|---|---|",
        ]
    )
    lines.extend(
        f"| {name} | {result} | 없음 |"
        for name, result in summary["hypotheses"].items()
    )
    (PUBLIC / "report.md").write_text("\n".join(lines) + "\n")
    print(json.dumps(summary, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
