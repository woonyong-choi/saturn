"""수집 결과와 원응답의 내용·사용량·session 정보를 대조한다."""

from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/recall-selection"


def module(path: Path) -> Any:
    spec = importlib.util.spec_from_file_location(path.stem, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def verify() -> None:
    raw = module(PUBLIC.parent / "real-context-replay/scripts/04-verify.py")
    metadata = module(PUBLIC.parent / "context-recall/scripts/05-archive.py")
    phases = {}
    for run in sorted(PRIVATE.iterdir()):
        if not (run / "calls-plan.json").exists():
            continue
        for relative, digest in json.loads((run / "seal.json").read_text()).items():
            if hashlib.sha256((run / relative).read_bytes()).hexdigest() != digest:
                raise RuntimeError(f"sealed input changed: {relative}")
        reuse_seal = run / "reuse-seal.json"
        if reuse_seal.exists():
            for relative, digest in json.loads(reuse_seal.read_text()).items():
                if hashlib.sha256((run / relative).read_bytes()).hexdigest() != digest:
                    raise RuntimeError("reuse mapping changed")
        counts = dict(planned=0, completed=0, failed=0, missing=0)
        for job in json.loads((run / "calls-plan.json").read_text()):
            for provider in ["claude", "codex"]:
                counts["planned"] += 1
                reuse_path = run / "reuse.json"
                reuse = (
                    json.loads(reuse_path.read_text()) if reuse_path.exists() else {}
                )
                phase = reuse.get(f"{job['id']}-{job['arm']}", run.name)
                folder = (
                    PRIVATE / phase / "raw" / f"{provider}-{job['id']}-{job['arm']}"
                )
                if not (folder / "result.json").exists():
                    counts["missing"] += 1
                    continue
                if (folder / "context.txt").read_text() != job["context"] or (
                    folder / "question.txt"
                ).read_text() != job["question"]:
                    raise RuntimeError(f"provider input changed: {folder.name}")
                result = json.loads((folder / "result.json").read_text())
                if result["status"] != "ok":
                    counts["failed"] += 1
                    continue
                usage, answer = raw.raw_response(folder, provider)
                if usage != result["usages"] or answer != result["text"]:
                    raise RuntimeError(f"raw response mismatch: {folder.name}")
                metadata.verify_metadata(folder, provider, result)
                counts["completed"] += 1
        phases[run.name] = counts
    output = dict(phases=phases)
    (PUBLIC / "results/verification.json").write_text(
        json.dumps(output, indent=2) + "\n"
    )
    print(json.dumps(output))


if __name__ == "__main__":
    verify()
