"""고정된 대조 조건을 실제 engine에서 실행하고 실패도 원자료로 남긴다."""
from __future__ import annotations

import argparse
import ast
import concurrent.futures
import gzip
import hashlib
import importlib.util
import json
import os
import random
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

EXP = Path(__file__).resolve().parents[1]
REPO = EXP.parents[2]
BASE = EXP.parent / 'context-net-effect' / 'scripts'
sys.path.insert(0, str(BASE))
import engine_driver
import fixtures
import plan as original_plan

ARMS = ('provider', 'rrf', 'rrf_lookup', 'jev_lookup', 'rescue')
PROVIDERS = ('claude', 'codex')
SEEDS = (101, 102, 103, 104)
ORDER_SEED = 58701
THRESHOLD = 35000
MODELS = {'claude': 'claude/haiku', 'codex': 'codex/gpt-5.6-luna'}
TABLES = ('chats', 'inputs', 'sessions', 'usage', 'judgments', 'handoff_packets',
          'handoff_packet_items', 'evidence_lookups', 'events')


def extract(db: Path) -> dict:
    con = sqlite3.connect(f'file:{db}?mode=ro', uri=True)
    con.row_factory = sqlite3.Row
    out = {table: [dict(r) for r in con.execute(f'SELECT * FROM {table} ORDER BY 1')]
           for table in TABLES}
    out['runs_meta'] = [dict(r) for r in con.execute(
        'SELECT id,chat_id,input_id,task_id,session_id,provider,started_at,ended_at,end_kind FROM runs ORDER BY id')]
    con.close()
    return out


def build(base: Path, seed: int) -> tuple[dict, str, dict]:
    facts = fixtures.build(base, seed)
    incident = (base / 'feed/incident.txt').read_text()
    # 정답 파일은 provider 시작 전에 제거하고 채점 값은 수집기 메모리에 둔다.
    (base / 'facts.json').unlink()
    (base / 'hidden/check.py').unlink()
    for part in range(4):
        lines = [f'2026-10-01T{part:02}:00:{i % 60:02}Z service=archive record={i:04} '
                 f'trace={hashlib.sha256(f"{seed}-{part}-{i}".encode()).hexdigest()} status=ok elapsed_ms={i % 231}\n'
                 for i in range(180)]
        (base / f'practice/docs/history-{part}.log').write_text(''.join(lines))
    tests = {str(f.relative_to(base / 'practice')): hashlib.sha256(f.read_bytes()).hexdigest()
             for f in (base / 'practice/tests').glob('*.py')}
    return facts, incident, tests


def grade(base: Path, facts: dict, tests: dict) -> dict:
    values = {}
    for name in ('config', 'report'):
        try:
            tree = ast.parse((base / f'practice/app/{name}.py').read_text())
            for node in tree.body:
                if isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name):
                    try:
                        values[node.targets[0].id] = ast.literal_eval(node.value)
                    except (ValueError, TypeError):
                        values[node.targets[0].id] = None
        except (SyntaxError, FileNotFoundError):
            values['parse_error'] = True
    result = subprocess.run(['python3', '-m', 'unittest', '-q'], cwd=base / 'practice',
                            capture_output=True, text=True, timeout=60)
    intact = all((base / 'practice' / n).exists() and
                 hashlib.sha256((base / 'practice' / n).read_bytes()).hexdigest() == h
                 for n, h in tests.items())
    return {'header_ok': values.get('REQUEST_HEADER') == facts['header_new'],
            'timeout_ok': values.get('TIMEOUT_SECONDS') == facts['timeout'],
            'retries_ok': values.get('RETRIES') == 2,
            'slow_ok': values.get('SLOW_ENDPOINT') == facts['slow_endpoint'],
            'visible_tests_ok': result.returncode == 0 and intact,
            'values': values, 'tests_unchanged': intact,
            'test_stdout': result.stdout, 'test_stderr': result.stderr}


def record(result: dict, key: str) -> None:
    raw = json.dumps(result, ensure_ascii=False, sort_keys=True).encode()
    if key.encode() in raw:
        raise RuntimeError('credential appeared in captured output; collection stopped')
    dest = EXP / 'data/raw' / (result['trial'] + '.json.gz')
    with dest.open('xb') as stream:
        with gzip.GzipFile(fileobj=stream, mode='wb', mtime=0) as zipped:
            zipped.write(raw)


def trial(provider: str, seed: int, arm: str, key: str) -> dict:
    ident = f'{provider}-{seed}-{arm}'
    base = REPO / '.runtime/r587' / f'{"a" if provider == "claude" else "x"}{seed}{ARMS.index(arm)}'
    facts, incident, tests = build(base, seed)
    chat = engine_driver.Chat(REPO / 'target/release', base / 'h', base / 'practice', key)
    options = ['permission.mode=full', 'model.mode=manual', f'model.default={MODELS[provider]}',
               f'context.evidence.lookup={str(arm.endswith("lookup")).lower()}']
    if arm == 'provider':
        options.append('context.mode=provider')
    turns = []
    grades = {}
    result = {'trial': ident, 'provider': provider, 'seed': seed, 'arm': arm,
              'facts': facts, 'turns': turns, 'grades': grades, 'complete': False,
              'started_unix': time.time(), 'source_commit': subprocess.check_output(
                  ['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()}

    def say(label: str, text: str, extra: list[str] | None = None) -> bool:
        rec = chat.send(text, options + (extra or []), timeout=300)
        rec['label'] = label
        turns.append(rec)
        print(f'{ident} {label} {rec["status"]}', flush=True)
        return rec['status'] == 'ok'

    try:
        for i, text in enumerate(original_plan.SETUP_TURNS, 1):
            if not say(f's{i}', text.format(**facts)):
                return result
        for part in range(4):
            if not say(f'load{part}', f'`cat docs/history-{part}.log`를 실행해 보관 로그를 읽어. 파일을 고치지 말고 읽음이라고만 답해.'):
                return result
        selector = 'jev' if arm == 'jev_lookup' else 'rrf'
        extra = [] if arm == 'provider' else [f'provider.{provider}.context.t_abs={THRESHOLD}',
                                             f'context.select.packet={selector}']
        if not say('boundary', original_plan.BOUNDARY_TURN, extra):
            return result
        result['boundary_snapshot'] = extract(base / 'h/saturn.db')
        if arm == 'provider' and not say('compact', '/compact'):
            return result
        for label, text in original_plan.FOLLOW_UPS:
            if arm == 'rescue' and label == 'f1':
                # 이미 읽은 근거를 후속 질문에 보강하는 진단 조건이다. 제품 효과에 포함하지 않는다.
                text = ('[진단용 원문 보강]\n' + incident + '\n정정된 사용자 규칙: 요청 추적 헤더 이름은 '
                        + facts['header_new'] + '\n지표 원문에서 가장 큰 p99 행의 endpoint: '
                        + facts['slow_endpoint'] + '\n[작업]\n' + text)
            if not say(label, text.format(**facts)):
                return result
            if label in ('f1', 'f3'):
                grades[label] = grade(base, facts, tests)
        result['complete'] = True
        return result
    finally:
        result['ended_unix'] = time.time()
        result['engine_pids_stopped'] = chat.stop_engine()
        result['engines_remaining'] = chat.engine_pids()
        db = base / 'h/saturn.db'
        result['store'] = extract(db) if db.exists() else {}
        record(result, key)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument('--pilot', action='store_true')
    parser.add_argument('--workers', type=int, default=2)
    args = parser.parse_args()
    if not 1 <= args.workers <= 2:
        raise ValueError('workers must be one or two')
    key = engine_driver.router_key()
    pairs = [(p, s, a) for p in PROVIDERS for s in SEEDS for a in ARMS]
    random.Random(ORDER_SEED).shuffle(pairs)
    if args.pilot:
        pairs = [('claude', 9901, 'rrf_lookup'), ('codex', 9901, 'rrf_lookup')]
    manifest = EXP / ('pilot-order.json' if args.pilot else 'order.json')
    manifest.write_text(json.dumps(pairs, indent=2) + '\n')
    failed = False
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = []
        for index, (p, s, a) in enumerate(pairs):
            if (EXP / 'data/raw' / f'{p}-{s}-{a}.json.gz').exists():
                continue
            if index == 1:
                time.sleep(15)
            futures.append(pool.submit(trial, p, s, a, key))
        for f in concurrent.futures.as_completed(futures):
            try:
                f.result()
            except Exception as error:
                failed = True
                # 키를 포함할 수 있는 예외 메시지는 출력하지 않는다.
                print('collector exception:', type(error).__name__, flush=True)
    return int(failed)


if __name__ == '__main__':
    sys.exit(main())
