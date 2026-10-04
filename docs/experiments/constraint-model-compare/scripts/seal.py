"""기대 응답이 전부 저장된 뒤 원자료를 한 번만 봉인한다."""

from __future__ import annotations

import hashlib

from runtime import PRIVATE, PUBLIC, read, rows, write

METADATA = (
    "samples.json",
    "sampling.json",
    "census.json",
    "sample-seal.json",
    "gold.json",
    "gold-seal.json",
    "calls.jsonl",
    "model-list.json",
    "claude-models.json",
    "deviations.json",
    "source-audit.json",
)


def required_raw_paths(calls: list[dict]) -> set[str]:
    return {*METADATA, *("raw/" + row["trial_id"] + ".json" for row in calls)}


def check_seals() -> None:
    required = {
        "sample-seal.json": {"samples.json", "sampling.json", "census.json"},
        "gold-seal.json": {"gold.json"},
        "raw-seal.json": required_raw_paths(rows(PRIVATE / "calls.jsonl")),
    }
    for filename, names in required.items():
        path = PRIVATE / filename
        if not path.exists():
            raise RuntimeError("collection seal missing: " + filename)
        manifest = read(path)
        if set(manifest) != names:
            raise RuntimeError("seal coverage mismatch: " + filename)
        for name, checksum in manifest.items():
            if hashlib.sha256((PRIVATE / name).read_bytes()).hexdigest() != checksum:
                raise RuntimeError("seal mismatch: " + name)


def main() -> None:
    samples = read(PRIVATE / "samples.json")
    calls = rows(PRIVATE / "calls.jsonl")
    expected = {
        f"query-{name}-{sample['sample_id']}-r{repeat}"
        for name in ("sonnet", "opus", "haiku", "astra", "sol", "terra", "luna", "jev")
        for sample in samples
        for repeat in ((1, 2, 3) if name == "jev" else (1,))
    }
    if not expected.issubset({r["trial_id"] for r in calls}):
        raise RuntimeError("cannot seal incomplete collection")
    receipts = [PRIVATE / "raw" / (row["trial_id"] + ".json") for row in calls]
    if not all(path.exists() for path in receipts):
        raise RuntimeError("cannot seal outstanding calls")
    files = [PRIVATE / name for name in required_raw_paths(calls)]
    checksums = {
        str(path.relative_to(PRIVATE)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(files)
    }
    target = PRIVATE / "raw-seal.json"
    if target.exists() and read(target) != checksums:
        raise RuntimeError("raw seal already exists and differs")
    if not target.exists():
        write(target, checksums)
    (PUBLIC / "data/SHA256SUMS").write_text(
        "".join(f"{checksum}  {name}\n" for name, checksum in checksums.items())
    )
    print("raw collection sealed")


if __name__ == "__main__":
    main()
