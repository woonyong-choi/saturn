"""아카이브 영수증과 현재 파일, tar 안의 파일을 모두 대조한다."""

import hashlib
import json
from pathlib import Path
import tarfile

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]


def verify() -> None:
    receipt = json.loads((PUBLIC / "data/archive.json").read_text())
    archive = ROOT / receipt["path"]
    manifest = PUBLIC / "data/SHA256SUMS"
    for path, expected in [
        (archive, receipt["sha256"]),
        (manifest, receipt["manifest_sha256"]),
    ]:
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise RuntimeError(f"digest mismatch: {path.name}")
    lines = manifest.read_text().splitlines()
    if len(lines) != receipt["files"]:
        raise RuntimeError("manifest count differs from receipt")
    with tarfile.open(archive) as bundle:
        for line in lines:
            expected, name = line.split("  ", 1)
            if hashlib.sha256((ROOT / name).read_bytes()).hexdigest() != expected:
                raise RuntimeError(f"current file changed: {name}")
            if hashlib.sha256(bundle.extractfile(name).read()).hexdigest() != expected:
                raise RuntimeError(f"archived file changed: {name}")
    print(json.dumps(dict(verified_files=len(lines), archive=receipt["path"])))


if __name__ == "__main__":
    verify()
