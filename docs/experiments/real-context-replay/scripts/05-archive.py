"""연구 근거를 hash 목록과 검증한 로컬 보관 묶음으로 저장한다."""

from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import tarfile
from datetime import datetime, timezone
from pathlib import Path

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/real-context-replay"
VERIFICATION = ROOT / ".local/verification/cleanup-context"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def snapshot() -> None:
    folder = PRIVATE / "source-snapshot"
    folder.mkdir(exist_ok=True)
    head = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    (folder / "state.json").write_text(
        json.dumps({"head": head, "working_tree": "diff.patch"}, indent=2)
    )
    (folder / "diff.patch").write_bytes(
        subprocess.check_output(["git", "diff", "--binary", "HEAD"], cwd=ROOT)
    )
    names = subprocess.check_output(
        ["git", "diff", "--name-only"], cwd=ROOT, text=True
    ).splitlines()
    names += [
        "saturn-terminal/engine/src/handoff_replay.rs",
        "docs/experiments/constraint-registration-generalization/scripts/01-prepare.py",
        "docs/experiments/constraint-long-context/scripts/common.py",
        "docs/experiments/same-session-compaction/scripts/05-persistent.py",
    ]
    for name in names:
        source = ROOT / name
        if source.is_file():
            target = folder / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)


def evidence_paths() -> list[Path]:
    excluded = {PUBLIC / "data/SHA256SUMS", PUBLIC / "data/archive.json"}
    return sorted(
        path
        for folder in [PUBLIC, PRIVATE, VERIFICATION]
        for path in folder.rglob("*")
        if path.is_file()
        and path not in excluded
        and not {"__pycache__", ".pytest_cache"}.intersection(path.parts)
    )


def main() -> None:
    snapshot()
    files = evidence_paths()
    hashes = {str(path.relative_to(ROOT)): digest(path) for path in files}
    manifest = PUBLIC / "data/SHA256SUMS"
    manifest.write_text("".join(f"{value}  {name}\n" for name, value in hashes.items()))
    archives = ROOT / ".local/archives"
    archives.mkdir(exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    archive = archives / f"real-context-replay-{stamp}.tar.gz"
    if archive.exists():
        raise RuntimeError("archive already exists")
    with tarfile.open(archive, "w:gz") as bundle:
        for path in files + [manifest]:
            bundle.add(path, arcname=str(path.relative_to(ROOT)), recursive=False)
    with tarfile.open(archive, "r:gz") as bundle:
        for name, expected in hashes.items():
            stream = bundle.extractfile(name)
            if stream is None or hashlib.sha256(stream.read()).hexdigest() != expected:
                raise RuntimeError(f"archive verification failed: {name}")
    receipt = {
        "path": str(archive.relative_to(ROOT)),
        "sha256": digest(archive),
        "bytes": archive.stat().st_size,
        "payload_files": len(files),
        "manifest_sha256": digest(manifest),
        "verified_all_payload_bytes": True,
        "receipt_outside_bundle": True,
    }
    (PUBLIC / "data/archive.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
