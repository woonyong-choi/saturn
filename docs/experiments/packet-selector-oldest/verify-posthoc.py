"""수집 뒤 원자료를 독립 대조한다. 봉인된 실행·분석 코드는 수정하지 않는다."""

import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("oldest_run_verify", HERE / "scripts/run.py")
experiment = importlib.util.module_from_spec(spec)
spec.loader.exec_module(experiment)
experiment.verify()
collector = experiment.collector
manifest = collector.check_seal()
summary = json.loads((collector.RUN / "summary.json").read_text())

for index in range(12):
    source = json.loads((collector.RUN / "raw" / f"s{index:02}.json").read_text())
    files, answer, current = experiment.files(index)
    assert source["valid"] and current == "build.log"
    assert str(answer) not in collector.PROMPT
    assert all(manifest["fixtures"][str(index)]["file_hashes"][name] == collector.sha(body.encode()) for name, body in files.items())
    results = [json.loads(e["body"])["ToolResult"]["output"] for e in source["db"]["events"] if e["body"].startswith('{"ToolResult"')]
    assert all(body in results for body in files.values())
    raw = {arm: json.loads((collector.RUN / "raw" / f"t{index:02}{arm}.json").read_text()) for arm in ("R", "J")}
    assert raw["R"]["chat"] == raw["J"]["chat"] == source["chat"]
    for arm in ("R", "J"):
        record = raw[arm]
        assert not any((collector.RUN / f"t{index:02}{arm}" / "work").iterdir())
        packet, items, capture = experiment.processor.first_packet(record)
        assert packet and capture and packet["state"] == "Sent"
        assert collector.sha(capture["body"].encode()) == packet["body_hash"] == capture["body_hash"]
        assert len(capture["body"].encode()) == packet["body_bytes"] == capture["body_bytes"]
        assert not any(e["body"].startswith('{"ToolCall"') for e in record["db"]["events"] if e["seq"] > packet["chat_revision"])
        assert record["status"] == "ok" and all(x == "Done" for x in record["tasks"].values())
        assert summary["pairs"][index]["arms"][arm]["cost_usd"] is not None
    r, j = summary["pairs"][index]["arms"]["R"], summary["pairs"][index]["arms"]["J"]
    assert r["protected"] == j["protected"] and r["packet_hash"] != j["packet_hash"]
    assert sorted(r["competing"]) != sorted(j["competing"])
    assert r["answer"] != str(answer) and j["answer"] == str(answer)

assert summary["source_valid"] == summary["complete_pairs"] == summary["joint_set_contrast"] == 12
assert summary["success"] == {"R": 0, "J": 12}
print("verified: 12 exact source results, paired protected rows, outbound captures, no target tools, hidden numeric answers")
