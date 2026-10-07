"""동일 문단 참조의 대응 입력을 고정하고 실제 응답을 수집한다."""

from __future__ import annotations

import concurrent.futures
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import random
import re
import shutil
import subprocess
import sys
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/transmission-quotes"
RUN = PRIVATE / "formal"
OLD = ROOT / ".local/experiments/recall-selection/explicit"
NEW = {
    "input-handling": [
        (
            "attempts",
            "보내기 전 확정 실패일 때 처음 시도를 포함한 최대 전송 횟수는? 숫자만.",
            "3",
            "처음 시도를 포함해 3번",
        ),
        (
            "rejudge",
            "적용 직전 revision이 다를 때 다시 판단하는 횟수는? 숫자만.",
            "1",
            "한 번 다시 판단",
        ),
        (
            "confidence",
            "이 값 미만이면 충돌로 보지 않고 대기로 두는 확신도 기준은? 숫자만.",
            "0.6",
            "확신도가 0.6 미만",
        ),
        (
            "stop",
            "멈춤 요청 뒤 남은 프로세스 묶음에 중지 신호를 보내기까지 몇 초 기다리는가? 숫자만.",
            "10",
            "10초 뒤 남은 프로세스",
        ),
    ],
    "engine-lifecycle": [
        (
            "backup",
            "이관 직전 만든 백업은 며칠 뒤 지우는가? 숫자만.",
            "14",
            "14일 뒤 지운다",
        ),
        (
            "history",
            "Attach 시 HistoryChunk가 담는 끝 기록은 몇 단위인가? 숫자만.",
            "50",
            "끝 50단위",
        ),
        (
            "load",
            "LoadHistory가 한 번에 응답하는 단위 상한은? 숫자만.",
            "500",
            "한 번에 500단위",
        ),
        (
            "version",
            "Version 요청에 몇 초 안에 답이 없으면 옛 engine으로 보지 않고 오류로 끝내는가? 숫자만.",
            "5",
            "5초(초안) 안에 답",
        ),
    ],
}


def save(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def load(path: Path) -> Any:
    return json.loads(path.read_text())


def module(path: Path) -> Any:
    spec = importlib.util.spec_from_file_location(path.stem, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def expand(text: str) -> str:
    header = re.match(
        r'Quoted-text encoding: <(saturn-quote-\d+) id="N">[^\n]+\n\n', text
    )
    if header is None:
        return text
    tag = header[1]
    body = text[header.end() :]
    definitions = dict(re.findall(rf'<{tag} id="(\d+)">(.*?)</{tag}>', body, re.S))
    body = re.sub(
        rf'<{tag} id="(\d+)">(.*?)</{tag}>', lambda match: match[2], body, flags=re.S
    )
    return re.sub(rf'<{tag} ref="(\d+)"/>', lambda match: definitions[match[1]], body)


def query(questions: list[dict[str, Any]]) -> str:
    keys = ", ".join(q["id"] for q in questions)
    return (
        "앞 기록의 근거만으로 답하세요. 도구를 사용하지 마세요. 답이 없으면 unknown입니다. 키별 값이 문자열인 JSON 객체 하나만 답하세요.\n"
        f"각 문항 ID를 JSON 키로 사용하세요. 키를 바꾸거나 추가하지 마세요. 반드시 사용할 키: {keys}.\n"
        + "\n".join(f"{q['id']}: {q['question']}" for q in questions)
    )


def prepare_sources() -> tuple[list[dict], dict]:
    cases, questions = load(OLD / "cases.json"), load(OLD / "questions.json")
    provenance = []
    for name, items in NEW.items():
        source = ROOT / f"docs/design/{name}.md"
        text = source.read_text()
        destination = RUN / "sources" / source.name
        destination.parent.mkdir(exist_ok=True)
        shutil.copy2(source, destination)
        case_id = f"heldout-{name}"
        row = dict(
            seq=1,
            run=1,
            session=1,
            task=1,
            input=f"{name}.md 문서 내용을 기록해 줘.",
            end="Completed",
            at_ms=0,
            event={"Text": dict(agent=1, subagent=None, text=text)},
        )
        cases.append(dict(id=case_id, rows=[row]))
        questions[case_id] = []
        for key, prompt, answer, anchor in items:
            lines = [i for i, line in enumerate(text.splitlines(), 1) if anchor in line]
            if not lines:
                raise RuntimeError(f"gold source anchor absent: {name}/{key}")
            questions[case_id].append(dict(id=key, question=prompt, expected=[answer]))
            provenance.append(
                dict(
                    case=case_id,
                    question=key,
                    source=str(source.relative_to(ROOT)),
                    sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                    anchor=anchor,
                    lines=lines,
                )
            )
    save(RUN / "gold-provenance.json", provenance)
    save(RUN / "cases.json", cases)
    save(RUN / "questions.json", questions)
    save(
        RUN / "queries.json",
        [
            dict(
                id=c["id"],
                case=c["id"],
                query=query(questions[c["id"]]),
                max_chars=8000,
            )
            for c in cases
        ],
    )
    return cases, questions


def export(binary: Path, arm: str) -> None:
    for kind, test in [
        ("packets", "export_real_record_packets"),
        ("recall", "export_recalled_evidence"),
    ]:
        env = dict(
            os.environ,
            SATURN_REPLAY_INPUT=str(RUN / "cases.json"),
            SATURN_RECALL_QUERIES=str(RUN / "queries.json"),
            SATURN_REPLAY_OUTPUT=str(RUN / arm / kind),
        )
        with (RUN / f"{arm}-{kind}.log").open("x") as log:
            subprocess.run(
                [str(binary), test, "--ignored"],
                cwd=ROOT,
                env=env,
                stdout=log,
                stderr=log,
                check=True,
            )


def payload_checks() -> list[dict]:
    checks = []
    for kind in ["packets", "recall"]:
        for path in sorted((RUN / "baseline" / kind).glob("*.json")):
            baseline, candidate = load(path), load(RUN / "candidate" / kind / path.name)
            before, after = baseline.get("text") or "", candidate.get("text") or ""
            check = dict(
                kind=kind,
                name=path.name,
                baseline_chars=len(before),
                candidate_chars=len(after),
                identical=before == after,
                reversible=expand(after) == before,
            )
            checks.append(check)
            if not check["reversible"]:
                raise RuntimeError(f"roundtrip mismatch: {path.name}")
    save(RUN / "payload-checks.json", checks)
    return checks


def prepare() -> None:
    RUN.mkdir(exist_ok=False)
    cases, _ = prepare_sources()
    export(PRIVATE / "baseline/engine-tests", "baseline")
    export(PRIVATE / "candidate-engine-tests", "candidate")
    payload_checks()
    jobs = []
    for repeat in range(1, 4):
        for case in cases:
            for arm in ["baseline", "candidate"]:
                jobs.append(
                    dict(
                        id=f"{case['id']}-r{repeat}",
                        case=case["id"],
                        repeat=repeat,
                        arm=arm,
                        context=load(RUN / arm / "packets" / f"{case['id']}-4000.json")[
                            "text"
                        ],
                        question=load(RUN / arm / "recall" / f"{case['id']}.json")[
                            "input"
                        ],
                    )
                )
    random.Random(7120).shuffle(jobs)
    save(RUN / "calls-plan.json", jobs)
    (RUN / "raw").mkdir()
    (RUN / "work").mkdir()
    versions = {
        name: subprocess.check_output([name, "--version"], text=True).strip()
        for name in ["rustc", "claude", "codex"]
    }
    save(
        RUN / "env.json",
        dict(
            seed=7120,
            platform=platform.platform(),
            versions=versions,
            head=subprocess.check_output(
                ["git", "rev-parse", "HEAD"], text=True
            ).strip(),
        ),
    )
    for relative in [
        "saturn-terminal/core/src/sessions/quotes.rs",
        "saturn-terminal/core/src/sessions/packet.rs",
        "saturn-terminal/core/src/sessions/mod.rs",
        "saturn-terminal/engine/src/handoff_recall.rs",
        "saturn-terminal/engine/src/handoff_replay.rs",
        "docs/experiments/transmission-quotes/design.md",
        "docs/experiments/transmission-quotes/scripts/01-run.py",
    ]:
        destination = RUN / "code" / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / relative, destination)
    save(
        RUN / "seal.json",
        {
            str(p.relative_to(RUN)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in RUN.rglob("*")
            if p.is_file()
        },
    )


def collect() -> None:
    helper = module(PUBLIC.parent / "real-context-replay/scripts/02-collect.py")
    helper.PRIVATE = RUN
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        futures = [
            pool.submit(helper.run_provider, provider, load(RUN / "calls-plan.json"))
            for provider in ["claude", "codex"]
        ]
        for future in futures:
            future.result()


def verify() -> None:
    for relative, expected in load(RUN / "seal.json").items():
        if hashlib.sha256((RUN / relative).read_bytes()).hexdigest() != expected:
            raise RuntimeError(f"sealed file changed: {relative}")
    raw = module(PUBLIC.parent / "real-context-replay/scripts/04-verify.py")
    meta = module(PUBLIC.parent / "context-recall/scripts/05-archive.py")
    counts = dict(planned=0, ok=0, failed=0, missing=0)
    for job in load(RUN / "calls-plan.json"):
        for provider in ["claude", "codex"]:
            counts["planned"] += 1
            folder = RUN / "raw" / f"{provider}-{job['id']}-{job['arm']}"
            if not (folder / "result.json").exists():
                counts["missing"] += 1
                continue
            if (folder / "context.txt").read_text() != job["context"] or (
                folder / "question.txt"
            ).read_text() != job["question"]:
                raise RuntimeError("actual input differs from plan")
            result = load(folder / "result.json")
            if result["status"] != "ok":
                counts["failed"] += 1
                continue
            usage, answer = raw.raw_response(folder, provider)
            if usage != result["usages"] or answer != result["text"]:
                raise RuntimeError("raw response mismatch")
            meta.verify_metadata(folder, provider, result)
            counts["ok"] += 1
    save(PUBLIC / "results/verification.json", counts)
    print(json.dumps(counts))


if __name__ == "__main__":
    os.umask(0o077)
    {"prepare": prepare, "collect": collect, "verify": verify}[sys.argv[1]]()
