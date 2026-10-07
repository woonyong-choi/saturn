"""실행 계획·입력·코드·검증 로그를 보관하고 내용 해시를 대조한다."""

from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import sys
import tarfile

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/joint-context-delivery"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def archive() -> None:
    paths = sorted(
        p
        for base in [
            PRIVATE,
            PUBLIC,
            ROOT / ".local/verification/joint-context-delivery",
        ]
        for p in base.rglob("*")
        if p.is_file()
        and not p.is_symlink()
        and "__pycache__" not in p.parts
        and p.name not in {"SHA256SUMS", "archive.json"}
    )
    paths.extend(
        PUBLIC.parent / "real-context-replay/scripts" / name
        for name in ["02-collect.py", "03-analyze.py", "04-verify.py"]
    )
    paths.extend(
        [
            PUBLIC.parent / "same-session-compaction/scripts/05-persistent.py",
            PUBLIC.parent / "context-recall/scripts/05-archive.py",
        ]
    )
    manifest = PUBLIC / "data/SHA256SUMS"
    manifest.write_text(
        "".join(f"{digest(p)}  {p.relative_to(ROOT)}\n" for p in sorted(set(paths)))
    )
    target = (
        ROOT
        / ".local/archives"
        / f"joint-context-delivery-{datetime.now(timezone.utc):%Y%m%dT%H%M%SZ}.tar.gz"
    )
    with tarfile.open(target, "x:gz") as bundle:
        for path in sorted(set(paths + [manifest])):
            bundle.add(path, arcname=str(path.relative_to(ROOT)))
    receipt = dict(
        path=str(target.relative_to(ROOT)),
        sha256=digest(target),
        files=len(manifest.read_text().splitlines()),
        manifest_sha256=digest(manifest),
    )
    (PUBLIC / "data/archive.json").write_text(json.dumps(receipt, indent=2) + "\n")
    verify()


def verify() -> None:
    receipt = json.loads((PUBLIC / "data/archive.json").read_text())
    target, manifest = ROOT / receipt["path"], PUBLIC / "data/SHA256SUMS"
    if (
        digest(target) != receipt["sha256"]
        or digest(manifest) != receipt["manifest_sha256"]
    ):
        raise RuntimeError("archive or manifest hash mismatch")
    rows = manifest.read_text().splitlines()
    if len(rows) != receipt["files"]:
        raise RuntimeError("archive file count mismatch")
    with tarfile.open(target) as bundle:
        for line in rows:
            expected, name = line.split("  ", 1)
            if (
                digest(ROOT / name) != expected
                or hashlib.sha256(bundle.extractfile(name).read()).hexdigest()
                != expected
            ):
                raise RuntimeError(f"archived file hash mismatch: {name}")
    print(json.dumps(dict(verified_files=len(rows), archive=receipt["path"])))


if __name__ == "__main__":
    os.umask(0o077)
    {"archive": archive, "verify": verify}[sys.argv[1]]()
