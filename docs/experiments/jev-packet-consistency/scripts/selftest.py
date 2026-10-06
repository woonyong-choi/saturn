"""Offline checks for response parsing and an unfinished send ledger."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import dev_replay
import shared


class PacketConsistencyTests(unittest.TestCase):
    def test_ordered_choice_and_invalid_probability(self) -> None:
        request = {
            "questions": {"call_4_keep": {}, "result_4_keep": {}, "call_2_keep": {}}
        }
        reply = {
            "answers": {
                "call_4_keep": {"noul": 0.7},
                "result_4_keep": {"noul": 0.2},
                "call_2_keep": {"noul": 0.7},
            }
        }
        self.assertEqual(
            shared.parse_selection(json.dumps(reply), json.dumps(request), 1),
            ("ok", [2]),
        )
        reply["answers"]["call_2_keep"]["noul"] = float("nan")
        self.assertEqual(
            shared.parse_selection(json.dumps(reply), json.dumps(request), 1),
            ("probability", None),
        )

    def test_started_request_without_end_is_failed_and_not_complete(self) -> None:
        request = json.dumps(
            {"model": "test", "questions": {"call_2_keep": {"type": "noul"}}}
        )
        source = {
            "id": "t00J",
            "request": request,
            "request_sha256": "hash",
            "source_sha256": "source",
            "k": 1,
            "applied_ids": [2],
            "recorded_response": None,
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "raw.jsonl"
            start = {
                "event": "start",
                "id": "t00J",
                "rep": 1,
                "request_sha256": "hash",
                "source_sha256": "source",
            }
            path.write_text(json.dumps(start) + "\n", encoding="utf-8")
            summary = dev_replay.analyze([source], path)
        self.assertEqual(summary["replays_started"], 1)
        self.assertEqual(summary["replays_finished"], 0)
        self.assertEqual(summary["parse_ok"], {"k": 0, "n": 5})
        self.assertEqual(summary["same_selection"]["k"], 0)

    def test_crash_after_start_never_resends(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            key_file = Path(directory) / "key"
            output = Path(directory) / "ledger.jsonl"
            key_file.write_text("secret", encoding="utf-8")
            trial = {"id": "case", "rep": 1, "request_text": "{}"}
            with patch.object(shared, "send", side_effect=RuntimeError("crash")):
                with self.assertRaises(RuntimeError):
                    shared.collect_trials([trial], ("id", "rep"), key_file, output)
            with patch.object(shared, "send") as send:
                shared.collect_trials([trial], ("id", "rep"), key_file, output)
                send.assert_not_called()
            starts, ends = shared.ledger(output, ("id", "rep"))
            self.assertEqual(len(starts), 1)
            self.assertEqual(len(ends), 0)

    def test_development_parser_requires_all_questions(self) -> None:
        request = json.dumps({"questions": {"call_2_keep": {}}})
        valid = json.dumps({"answers": {"call_2_keep": {"noul": 0.5}}})
        missing = json.dumps({"answers": {}})
        self.assertEqual(shared.parse_selection(valid, request, 1), ("ok", [2]))
        self.assertEqual(
            shared.parse_selection(missing, request, 1), ("question_mismatch", None)
        )


if __name__ == "__main__":
    unittest.main()
