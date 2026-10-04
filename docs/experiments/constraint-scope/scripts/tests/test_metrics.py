"""실험의 분모·작업 범위 필터·형식 실패 경계를 검증한다."""

from __future__ import annotations

import unittest
from pathlib import Path

from metrics import evaluate, judge_difference, rate
from runtime import load_module

SCRIPTS = Path(__file__).resolve().parents[1]

PROCESS = load_module("processing_test", SCRIPTS / "04-process.py")
ANALYSIS = load_module("analysis_test", SCRIPTS / "05-analyze.py")


def row(label: str, probability: float | None) -> dict:
    return dict(gold=label, probability=probability, valid=probability is not None)


class MetricsTests(unittest.TestCase):
    def test_design_failure_and_uncertainty_use_distinct_denominators(self) -> None:
        result = evaluate(
            [
                row("constraint", 0.9),
                row("constraint", None),
                row("not_constraint", 0.75),
                row("uncertain", 0.95),
            ],
            0.9,
            0.7,
        )
        self.assertEqual((result["precision"]["k"], result["precision"]["n"]), (1, 1))
        self.assertEqual((result["recall"]["k"], result["recall"]["n"]), (1, 2))
        self.assertEqual((result["ask_rate"]["k"], result["ask_rate"]["n"]), (1, 4))
        self.assertEqual(result["unresolved_auto"], 1)
        self.assertEqual(result["failed"], 1)

    def test_design_empty_prediction_is_not_perfect_precision(self) -> None:
        self.assertIsNone(evaluate([row("constraint", 0.1)], 0.9)["precision"]["value"])
        self.assertIsNone(rate(0, 0)["ci"])

    def test_design_scope_gate_boundary_and_invalid_probability(self) -> None:
        record = dict(
            status="ok",
            request={"questions": {"is_constraint": {}, "task_only": {}}},
            response={
                "model": "jev-1.13.0",
                "answers": {
                    "is_constraint": {"type": "noul", "noul": 0.9},
                    "task_only": {"type": "noul", "noul": 0.5},
                },
            },
        )
        self.assertEqual(PROCESS.jev_value(record), (0, 0.5))
        record["response"]["answers"]["task_only"]["noul"] = 0.49
        self.assertEqual(PROCESS.jev_value(record), (0.9, 0.49))
        record["response"]["answers"]["is_constraint"]["noul"] = float("nan")
        self.assertEqual(PROCESS.jev_value(record), (None, None))

    def test_design_minimum_auto_count_prevents_tiny_candidate(self) -> None:
        data = [row("constraint", 0.95)] * 9 + [row("not_constraint", 0.1)] * 21
        self.assertFalse(ANALYSIS.make_policy(data, "scope", 0.9, 0.7)["point_pass"])
        data.append(row("constraint", 0.95))
        self.assertTrue(ANALYSIS.make_policy(data, "scope", 0.9, 0.7)["point_pass"])

    def test_design_undefined_bootstrap_blocks_adoption(self) -> None:
        comparison = dict(value=0.1, ci=[0.01, 0.2], undefined_cluster=251)
        self.assertEqual(judge_difference([comparison]), "보류")
        comparison["undefined_cluster"] = 0
        self.assertEqual(judge_difference([comparison]), "채택")


if __name__ == "__main__":
    unittest.main()
