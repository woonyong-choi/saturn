"""provider 기록의 전달 패킷을 engine 해시와 대조해 원문을 보존한다."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
from pathlib import Path

import collect


def texts(event: dict) -> list[str]:
    message = event.get("message", {})
    if not isinstance(message, dict):
        message = {}
    if event.get("type") == "response_item":
        message = event.get("payload", {})
    if message.get("role") != "user":
        return []
    content = message.get("content", [])
    if isinstance(content, str):
        return [content]
    pieces = [p.get("text", "") for p in content if isinstance(p, dict)]
    return ["".join(pieces), *pieces]


def inspect_packets(capture: bool = False) -> list[dict]:
    cache = collect.EXP / "data/packets"
    cache.mkdir(exist_ok=True)
    files = sorted((collect.EXP / "data/raw").glob("*.gz")) + sorted(
        (collect.EXP / "data/followup").glob("*.gz")
    )
    native = None
    if capture:
        native = list((Path.home() / ".claude/projects").glob("*/*.jsonl")) + list(
            (Path.home() / ".codex/sessions").rglob("*.jsonl")
        )
        for runtime in ("r587", "r587f"):
            native.extend((collect.REPO / ".runtime" / runtime).rglob("*.jsonl"))
    key = collect.engine_driver.router_key() if capture else None
    rows = []
    for path in files:
        raw = json.load(gzip.open(path, "rt"))
        if raw["seed"] not in collect.SEEDS:
            continue
        for packet in raw.get("store", {}).get("handoff_packets", []):
            dest = cache / f"{raw['trial']}-{packet['id']}.txt.gz"
            body = gzip.decompress(dest.read_bytes()) if dest.exists() else None
            if body is None and native is not None and packet.get("provider_session"):
                for source in native:
                    if packet["provider_session"] not in source.name:
                        continue
                    for line in source.read_bytes().splitlines():
                        try:
                            event = json.loads(line)
                        except json.JSONDecodeError:
                            continue
                        for text in texts(event):
                            encoded = text.encode()
                            if (
                                hashlib.sha256(encoded).hexdigest()
                                == packet["body_hash"]
                            ):
                                body = encoded
                                break
                        if body is not None:
                            break
                    if body is not None:
                        break
                if body is not None:
                    if key.encode() in body:
                        raise RuntimeError("credential in packet; capture stopped")
                    with dest.open("xb") as stream:
                        with gzip.GzipFile(
                            fileobj=stream, mode="wb", mtime=0
                        ) as zipped:
                            zipped.write(body)
            if (
                body is not None
                and hashlib.sha256(body).hexdigest() != packet["body_hash"]
            ):
                raise ValueError("captured packet hash mismatch")
            row = {
                "trial": raw["trial"],
                "phase": raw.get("phase", "formal"),
                "provider": raw["provider"],
                "arm": raw["arm"],
                "packet_id": packet["id"],
                "body_hash": packet["body_hash"],
                "matched": body is not None,
                "corrected_header_present": raw["facts"]["header_new"].encode() in body
                if body is not None
                else None,
                "lookup_hint_present": b"saturn evidence read" in body
                if body is not None
                else None,
            }
            rows.append(row)
    return rows


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    rows = inspect_packets(args.capture)
    output = collect.EXP / "results/packet-audit.json"
    text = json.dumps(rows, sort_keys=True, indent=2) + "\n"
    if args.verify:
        if not output.exists() or output.read_text() != text:
            raise ValueError("packet audit differs")
    else:
        output.write_text(text)
    print("packets", len(rows), "matched", sum(r["matched"] for r in rows))


if __name__ == "__main__":
    main()
