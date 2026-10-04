"""CLI 진단을 도구 실행과 구분하고 router 확신 대체를 검증한다."""

from __future__ import annotations

import json
import unittest

from protocol import OPTIONS, normalize_choice
from runtime import response_text


class ContractTests(unittest.TestCase):
    # design.md의 도구 사용 거절은 CLI 진단 이벤트에 적용하지 않는다.
    def test_diagnostics_preserve_answer_and_tools_remain_detected(self) -> None:
        for item_type, expected in [("error", 0), ("mcp_tool_call", 1)]:
            with self.subTest(item_type=item_type):
                events = [
                    {"type": "item.completed", "item": {"type": item_type}},
                    {
                        "type": "item.completed",
                        "item": {"type": "agent_message", "text": '{"ok":true}'},
                    },
                ]
                record = {
                    "status": "ok",
                    "kind": "codex",
                    "stdout": "\n".join(json.dumps(e) for e in events),
                }
                text, metadata = response_text(record)
                self.assertEqual(text, '{"ok":true}')
                self.assertEqual(metadata["tool_events"], expected)

    # router 설계의 낮은 확신 대체와 허용 후보 선택 계약.
    def test_confidence_and_malformed_answer(self) -> None:
        uncertain = {
            "answers": {"target_model": {"probabilities": {k: 1 / 12 for k in OPTIONS}}}
        }
        self.assertEqual(normalize_choice(uncertain)["effective"], "codex/gpt-6-sol")
        probabilities = {k: 0 for k in OPTIONS}
        probabilities.update({"claude/haiku": 0.8, "other": 0.2})
        certain = {"answers": {"target_model": {"probabilities": probabilities}}}
        self.assertEqual(normalize_choice(certain)["effective"], "claude/haiku")
        self.assertIsNone(normalize_choice({"answers": []}))


if __name__ == "__main__":
    unittest.main()
