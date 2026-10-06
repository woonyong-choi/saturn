"""원자료와 봉인을 독립 재계산하고 누락·불일치·재전송을 거절한다."""

import importlib.util
import json
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("selector_process", BASE / "scripts/02-process.py")
process = importlib.util.module_from_spec(spec)
spec.loader.exec_module(process)
collect = process.collect


def verify() -> None:
    manifest = collect.check_seal()
    assert manifest["seed"] == 54026006 and manifest["planned"] == 12
    ledger = collect.RUN / "ledger"
    for index in range(12):
        source = process.load(collect.RUN / "raw" / f"s{index:02}.json")
        if source and source["valid"]:
            files, _, _ = collect.source_files(index)
            assert all(source["seen"].values())
            for name in collect.NAMES:
                assert manifest["fixtures"][str(index)]["file_hashes"][name] == collect.sha(files[name].encode())
                assert (ledger / f"s{index:02}-{name}.json").exists()
        for arm in ("R", "J"):
            label = f"t{index:02}{arm}"
            record = process.load(collect.RUN / "raw" / f"{label}.json")
            if record is None:
                continue
            assert source and source["valid"] and (ledger / f"{label}.json").exists()
            assert record["chat"] == source["chat"]
            assert record["overrides"] == collect.OVERRIDES + ([['context.select.packet', 'jev']] if arm == "J" else [])
            assert all("TIMEOUT=" not in n.get("params", {}).get("text", "") for n in record["notes"])
            packet, _, capture = process.first_packet(record)
            assert packet is not None and capture is not None
            assert collect.sha(capture["body"].encode()) == packet["body_hash"] == capture["body_hash"]
            assert len(capture["body"].encode()) == packet["body_bytes"] == capture["body_bytes"]
            expected = "compact" if arm == "J" else "rank"
            assert packet["requested_selector"] == packet["actual_selector"] == expected and packet["selection_fallback"] is None
            assert not any(e["body"].startswith('{"ToolCall"') for e in record["db"]["events"] if e["seq"] > packet["chat_revision"])
    old = (collect.RUN / "summary.json").read_bytes()
    first = process.analyze()
    assert json.loads(old) == first
    assert json.dumps(first, ensure_ascii=False, sort_keys=True) == json.dumps(process.analyze(), ensure_ascii=False, sort_keys=True)
    print("verified: seal, ledger, source, captures, selectors, no target tools, deterministic analysis")


if __name__ == "__main__":
    verify()
