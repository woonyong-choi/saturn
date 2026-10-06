"""프로젝트 시작 저장소와 숨긴 사실을 시드로 만든다. 같은 시드는 같은 바이트를 낸다.

저장소는 `practice/`, 한 번만 읽히는 피드와 숨긴 검사는 저장소 밖(`feed/`, `hidden/`)에 둔다.
"""

from __future__ import annotations

import json
import random
import subprocess
from pathlib import Path

HEADERS = [
    "X-Trace-Key", "X-Call-Token", "X-Flow-Ref", "X-Hop-Tag", "X-Span-Code", "X-Lane-Key",
    "X-Pipe-Ref", "X-Beam-Tag", "X-Route-Key", "X-Link-Token", "X-Wave-Ref", "X-Gate-Tag",
    "X-Node-Key", "X-Port-Ref", "X-Sync-Tag", "X-Pulse-Key",
]
ENDPOINTS = [
    "/v2/orders/export", "/v2/users/search", "/v2/invoices/batch", "/v2/catalog/sync",
    "/v2/reports/daily", "/v2/carts/merge", "/v2/refunds/list", "/v2/stock/audit",
    "/v2/tickets/bulk", "/v2/events/replay", "/v2/pricing/rules", "/v2/shipments/track",
    "/v2/coupons/scan", "/v2/billing/ledger", "/v2/devices/pair", "/v2/sessions/purge",
]
FILLER_ENDPOINTS = [
    "/v2/health", "/v2/login", "/v2/logout", "/v2/profile", "/v2/orders", "/v2/orders/{id}",
    "/v2/users", "/v2/users/{id}", "/v2/items", "/v2/items/{id}", "/v2/cart", "/v2/cart/items",
    "/v2/pay", "/v2/pay/confirm", "/v2/search", "/v2/notify", "/v2/export", "/v2/import",
    "/v2/settings", "/v2/audit", "/v2/tags", "/v2/tags/{id}", "/v2/files", "/v2/files/{id}",
    "/v2/hooks", "/v2/hooks/{id}", "/v2/keys", "/v2/keys/{id}", "/v2/teams", "/v2/teams/{id}",
    "/v2/roles", "/v2/roles/{id}", "/v2/plans", "/v2/plans/{id}", "/v2/usage", "/v2/limits",
    "/v2/queue", "/v2/queue/{id}", "/v2/jobs", "/v2/jobs/{id}", "/v2/status", "/v2/version",
    "/v2/feeds", "/v2/feeds/{id}", "/v2/locks", "/v2/locks/{id}", "/v2/mail", "/v2/mail/{id}",
    "/v2/rates", "/v2/zones", "/v2/zones/{id}", "/v2/maps", "/v2/maps/{id}", "/v2/stats",
    "/v2/stats/{id}", "/v2/ping", "/v2/echo", "/v2/trace", "/v2/trace/{id}",
]

CONFIG = '''"""서비스 설정."""

REQUEST_HEADER = "X-Req-Id"
TIMEOUT_SECONDS = 5
RETRIES = 2
'''

CLIENT = '''"""업스트림 호출 클라이언트."""

from . import config


def build_headers(token):
    return {config.REQUEST_HEADER: token, "Accept": "application/json"}


class Upstream:
    def __init__(self, transport):
        self.transport = transport
        self.calls = 0

    def call(self, endpoint, token):
        self.calls += 1
        headers = build_headers(token)
        last = None
        for _ in range(config.RETRIES + 1):
            try:
                return self.transport(endpoint, headers, config.TIMEOUT_SECONDS)
            except TimeoutError as exc:
                last = exc
        raise last
'''

STORE = '''"""메모리 저장소."""


class Store:
    def __init__(self):
        self.rows = {}

    def put(self, key, value):
        self.rows[key] = value

    def get(self, key, default=None):
        return self.rows.get(key, default)

    def delete(self, key):
        return self.rows.pop(key, None)

    def keys(self):
        return sorted(self.rows)
'''

REPORT = '''"""요약 보고."""

SLOW_ENDPOINT = "unknown"


def summarize(rows):
    return {"count": len(rows), "slow": SLOW_ENDPOINT}
'''

TEST_CLIENT = '''import unittest

from app import client, config


class ClientTest(unittest.TestCase):
    def test_header_uses_configured_name(self):
        self.assertEqual(client.build_headers("t"), {config.REQUEST_HEADER: "t", "Accept": "application/json"})

    def test_timeout_is_positive(self):
        self.assertGreater(config.TIMEOUT_SECONDS, 0)

    def test_retry_then_raise(self):
        def transport(endpoint, headers, timeout):
            raise TimeoutError(endpoint)

        upstream = client.Upstream(transport)
        with self.assertRaises(TimeoutError):
            upstream.call("/x", "t")


if __name__ == "__main__":
    unittest.main()
'''

TEST_STORE = '''import unittest

from app.store import Store


class StoreTest(unittest.TestCase):
    def test_put_get_delete(self):
        store = Store()
        store.put("a", 1)
        self.assertEqual(store.get("a"), 1)
        self.assertEqual(store.delete("a"), 1)
        self.assertIsNone(store.get("a"))


if __name__ == "__main__":
    unittest.main()
'''

TEST_REPORT = '''import unittest

from app import report


class ReportTest(unittest.TestCase):
    def test_summary_has_slow_text(self):
        self.assertIsInstance(report.summarize([])["slow"], str)


if __name__ == "__main__":
    unittest.main()
'''

HIDDEN_TEST = '''"""숨긴 검사. 저장소 밖에 있고 에이전트에게 보이지 않는다."""
import json
import subprocess
import sys

root = sys.argv[1]
code = (
    "import json, importlib;"
    "from app import config, report;"
    "print(json.dumps({'header': config.REQUEST_HEADER, 'timeout': config.TIMEOUT_SECONDS,"
    " 'retries': config.RETRIES, 'slow': report.SLOW_ENDPOINT}))"
)
got = subprocess.run([sys.executable, "-c", code], cwd=root, capture_output=True, text=True)
values = json.loads(got.stdout) if got.returncode == 0 else {}
tests = subprocess.run([sys.executable, "-m", "unittest", "-q"], cwd=root, capture_output=True, text=True)
print(json.dumps({"values": values, "visible_tests_ok": tests.returncode == 0}))
'''

INCIDENT_SH = '''#!/bin/sh
# 한 번만 읽히는 피드. 읽고 나면 지워진다.
FEED="$(dirname "$0")/../../feed/incident.txt"
if [ -f "$FEED" ]; then
  cat "$FEED"
  rm -f "$FEED"
else
  echo "incident feed already consumed (single-read token)"
fi
'''

METRICS_SH = '''#!/bin/sh
python3 "$(dirname "$0")/metrics.py"
'''

METRICS_PY = '''import json
import os

here = os.path.dirname(os.path.abspath(__file__))
with open(os.path.join(here, "metrics.json")) as handle:
    rows = json.load(handle)
print("endpoint,p50_ms,p99_ms,errors")
for row in rows:
    print("%s,%d,%d,%d" % (row["endpoint"], row["p50"], row["p99"], row["errors"]))
'''


def _readme(rng: random.Random) -> str:
    return (
        "# gateway-client\n\n"
        "업스트림 게이트웨이를 부르는 작은 클라이언트.\n\n"
        "## 설치\n\n표준 라이브러리만 쓴다. `python3 -m unittest -q`로 시험한다.\n\n"
        "## 설정\n\n"
        "| 키 | 값 | 뜻 |\n|---|---|---|\n"
        "| `REQUEST_HEADER` | `X-Req-Id` | 요청 추적 헤더 이름 |\n"
        "| `TIMEOUT_SECONDS` | `5` | 업스트림 호출 제한 시간 |\n"
        "| `RETRIES` | `2` | 시간 초과 때 다시 시도하는 횟수 |\n\n"
        "## 구조\n\n- `app/client.py` 업스트림 호출\n- `app/store.py` 메모리 저장소\n"
        "- `app/report.py` 요약 보고\n- `tools/` 운영 보조 스크립트\n\n"
        "## 운영 메모\n\n" + "".join(f"- 메모 {i}: 배포 {rng.randint(100, 999)}번 창구에서 확인한다.\n" for i in range(14))
    )


def _architecture(rng: random.Random) -> str:
    parts = ["# 구조\n\n호출은 `Upstream.call`이 맡고, 재시도는 설정의 `RETRIES`를 따른다.\n"]
    for i in range(1, 7):
        parts.append(
            f"\n## 절 {i}\n\n"
            + " ".join(
                f"단계 {i}.{j}에서 {rng.choice(['큐', '캐시', '게이트웨이', '저장소', '감시자'])}가 "
                f"{rng.choice(['요청을 받고', '결과를 쓰고', '상태를 확인하고', '재시도를 기록하고'])} "
                f"{rng.randint(10, 99)}ms 안에 다음 단계로 넘긴다."
                for j in range(1, 6)
            )
            + "\n"
        )
    return "".join(parts)


def facts_for(seed: int) -> dict:
    rng = random.Random(seed)
    return {
        "seed": seed,
        "ticket": "INC-%06d" % rng.randint(100000, 999999),
        "timeout": rng.choice([t for t in range(21, 48) if t != 5]),
        "header_new": HEADERS[seed % len(HEADERS)],
        "slow_endpoint": ENDPOINTS[(seed * 7 + 3) % len(ENDPOINTS)],
    }


def _write(path: Path, text: str, mode: int | None = None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    if mode is not None:
        path.chmod(mode)


def _git(root: Path, *args: str) -> None:
    subprocess.run(
        ["git", "-c", "user.email=exp@example.invalid", "-c", "user.name=exp", *args],
        cwd=root,
        check=True,
        capture_output=True,
    )


def build(trial_dir: Path, seed: int) -> dict:
    """`trial_dir/practice`, `feed`, `hidden`를 만들고 숨긴 사실을 돌려준다. 이미 있으면 거절한다."""
    if trial_dir.exists():
        raise FileExistsError(trial_dir)
    rng = random.Random(seed * 1009 + 17)
    facts = facts_for(seed)
    root = trial_dir / "practice"
    _write(root / "app" / "__init__.py", "")
    _write(root / "app" / "config.py", CONFIG)
    _write(root / "app" / "client.py", CLIENT)
    _write(root / "app" / "store.py", STORE)
    _write(root / "app" / "report.py", REPORT)
    _write(root / "tests" / "__init__.py", "")
    _write(root / "tests" / "test_client.py", TEST_CLIENT)
    _write(root / "tests" / "test_store.py", TEST_STORE)
    _write(root / "tests" / "test_report.py", TEST_REPORT)
    _write(root / "README.md", _readme(rng))
    _write(root / "docs" / "architecture.md", _architecture(rng))
    _write(root / "tools" / "incident.sh", INCIDENT_SH, 0o755)
    _write(root / "tools" / "dump-metrics.sh", METRICS_SH, 0o755)
    _write(root / "tools" / "metrics.py", METRICS_PY)
    names = list(FILLER_ENDPOINTS)
    rng.shuffle(names)
    rows = [{"endpoint": n, "p50": rng.randint(20, 180), "p99": rng.randint(200, 900), "errors": rng.randint(0, 9)} for n in names]
    slow = {"endpoint": facts["slow_endpoint"], "p50": rng.randint(150, 300), "p99": rng.randint(1500, 1900), "errors": rng.randint(0, 9)}
    rows.insert(len(rows) // 2, slow)
    _write(root / "tools" / "metrics.json", json.dumps(rows, indent=1))
    _write(root / ".gitignore", "__pycache__/\n*.pyc\n")
    # Claude Code의 자동 메모리가 세션 사이로 사실을 옮기지 않게 이 저장소 설정으로 끈다
    _write(root / ".claude" / "settings.json", '{"autoMemoryEnabled": false}\n')
    feed = (
        f"INCIDENT {facts['ticket']}  severity=high  service=gateway\n"
        "opened: 2026-09-30T02:11:09Z  owner: platform-oncall\n"
        "summary: upstream calls from app.client time out under load\n"
        f"root cause: gateway enforces a minimum client timeout of {facts['timeout']} seconds; "
        "a smaller TIMEOUT_SECONDS is rejected with 504\n"
        f"remediation: set TIMEOUT_SECONDS to {facts['timeout']} in app/config.py\n"
        "note: do not change RETRIES\n"
    )
    _write(trial_dir / "feed" / "incident.txt", feed)
    _write(trial_dir / "hidden" / "check.py", HIDDEN_TEST)
    (trial_dir / "facts.json").write_text(json.dumps(facts, sort_keys=True, indent=1))
    _git(root, "init", "-q")
    _git(root, "add", "-A")
    _git(root, "commit", "-qm", "chore: 초기 클라이언트")
    for i, msg in enumerate(["docs: 설정 절 추가", "fix: 재시도 횟수 기본값 조정", "chore: 운영 스크립트 추가", "docs: 구조 문서 보강"]):
        (root / "README.md").write_text((root / "README.md").read_text() + f"\n<!-- 변경 {i} -->\n")
        _git(root, "commit", "-qam", msg)
    return facts


def grade(trial_dir: Path) -> dict:
    """숨긴 검사로 지금 저장소의 값을 읽는다. 에이전트의 파일을 실행하는 것은 설정 값 읽기와 공개 시험뿐이다."""
    facts = json.loads((trial_dir / "facts.json").read_text())
    out = subprocess.run(
        ["python3", str(trial_dir / "hidden" / "check.py"), str(trial_dir / "practice")],
        capture_output=True,
        text=True,
        timeout=120,
    )
    got = json.loads(out.stdout) if out.returncode == 0 and out.stdout.strip() else {"values": {}, "visible_tests_ok": False}
    values = got["values"]
    return {
        "header_ok": values.get("header") == facts["header_new"],
        "timeout_ok": values.get("timeout") == facts["timeout"],
        "retries_ok": values.get("retries") == 2,
        "slow_ok": values.get("slow") == facts["slow_endpoint"],
        "visible_tests_ok": got["visible_tests_ok"],
        "values": values,
    }
