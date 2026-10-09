"""원응답 대조와 파일별 해시 검증을 거쳐 연구 자료를 별도 묶음으로 보존한다."""

from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tarfile
from datetime import datetime, timezone

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/context-recall"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_responses():
    path = PUBLIC.parent / "real-context-replay/scripts/04-verify.py"
    spec = importlib.util.spec_from_file_location("raw_verifier", path)
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    return {
        run.name: verify_run(run, helper)
        for run in PRIVATE.iterdir()
        if (run / "collection-seal.json").exists()
    }


def verify_run(run, helper):
    sealed = json.loads((run / "collection-seal.json").read_text())
    for name, expected in sealed.items():
        if sha(run / name) != expected:
            raise RuntimeError(f"collection input changed: {name}")
    jobs = json.loads((run / "calls-plan.json").read_text())
    checked = 0
    for provider in ["claude", "codex"]:
        for job in jobs:
            folder = run / "raw" / f"{provider}-{job['id']}-{job['arm']}"
            if folder.exists() and (
                (folder / "context.txt").read_text() != job["context"]
                or (folder / "question.txt").read_text() != job["question"]
            ):
                raise RuntimeError(f"input differs from sealed plan: {folder.name}")
            if (
                not (folder / "result.json").exists()
                and (run / "stopped.json").exists()
            ):
                continue
            result = json.loads((folder / "result.json").read_text())
            if result["status"] == "ok":
                usages, answer = helper.raw_response(folder, provider)
                if usages != result["usages"] or answer != result["text"]:
                    raise RuntimeError(
                        f"parsed result differs from raw response: {folder.name}"
                    )
                verify_metadata(folder, provider, result)
            checked += 1
    return checked


def verify_metadata(folder, provider, result):
    if provider == "claude":
        batches = [
            [json.loads(line) for line in (folder / name).read_text().splitlines()]
            for name in ["ready.jsonl", "answer.jsonl"]
        ]
        last = [
            next(row for row in reversed(batch) if row.get("type") == "result")
            for batch in batches
        ]
        if result["ready"] != last[0].get("result", "").strip() or result[
            "same_session"
        ] != (last[0].get("session_id") == last[1].get("session_id")):
            raise RuntimeError(
                f"session metadata differs from raw response: {folder.name}"
            )
        if result.get("model_usage", {}) != last[-1].get("modelUsage", {}):
            raise RuntimeError(
                f"model metadata differs from raw response: {folder.name}"
            )
    else:
        events = [
            json.loads(line)
            for line in (folder / "stdout.jsonl").read_text().splitlines()
        ]
        count = sum(
            row.get("type") == "item.completed"
            and row.get("item", {}).get("type") not in ("reasoning", "agent_message")
            for row in events
        )
        if result["tool_calls"] != count:
            raise RuntimeError(f"tool count differs from raw response: {folder.name}")


def archive():
    responses = verify_responses()
    data = PUBLIC / "data"
    data.mkdir(exist_ok=True)
    files = sorted(
        p
        for base in [PRIVATE, PUBLIC, ROOT / ".local/verification/context-recall"]
        for p in base.rglob("*")
        if p.is_file()
        and p.name not in ["archive.json", "SHA256SUMS"]
        and "__pycache__" not in p.parts
    )
    dependencies = list((PUBLIC.parent / "real-context-replay/scripts").rglob("*.py"))
    dependencies.extend(
        [
            PUBLIC.parent / "same-session-compaction/scripts/05-persistent.py",
            PUBLIC.parent
            / "constraint-registration-generalization/scripts/01-prepare.py",
        ]
    )
    files = sorted(set(files + dependencies))
    lines = [f"{sha(p)}  {p.relative_to(ROOT)}" for p in files]
    manifest = data / "SHA256SUMS"
    manifest.write_text("\n".join(lines) + "\n")
    target = (
        ROOT
        / ".local/archives"
        / f"context-recall-{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')}.tar.gz"
    )
    with tarfile.open(target, "x:gz") as tar:
        for path in [*files, manifest]:
            tar.add(path, arcname=str(path.relative_to(ROOT)), recursive=False)
    with tarfile.open(target, "r:gz") as tar:
        for line in lines:
            digest, name = line.split("  ", 1)
            if hashlib.sha256(tar.extractfile(name).read()).hexdigest() != digest:
                raise RuntimeError(f"archive readback mismatch: {name}")
    receipt = dict(
        path=str(target.relative_to(ROOT)),
        sha256=sha(target),
        files=len(files),
        responses=responses,
        manifest_sha256=sha(manifest),
    )
    (data / "archive.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt))


def verify():
    responses = verify_responses()
    receipt = json.loads((PUBLIC / "data/archive.json").read_text())
    if sha(ROOT / receipt["path"]) != receipt["sha256"]:
        raise RuntimeError("archive digest mismatch")
    manifest = PUBLIC / "data/SHA256SUMS"
    if sha(manifest) != receipt["manifest_sha256"]:
        raise RuntimeError("manifest digest mismatch")
    for line in manifest.read_text().splitlines():
        digest, name = line.split("  ", 1)
        if sha(ROOT / name) != digest:
            raise RuntimeError(f"file digest mismatch: {name}")
    print(json.dumps(dict(verified_files=receipt["files"], responses=responses)))


if __name__ == "__main__":
    {"archive": archive, "verify": verify}[sys.argv[1]]()
