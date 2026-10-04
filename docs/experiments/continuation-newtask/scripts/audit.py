"""사용자 역할로 저장된 내부 이벤트와 오염된 직전 입력 쌍을 별도로 제외한다."""

from __future__ import annotations

import re
from collections import Counter

from runtime import PRIVATE, read, sha, write

INTERNAL_TAGS = (
    "heartbeat",
    "codex_internal_context",
    "codex_delegation",
    "realtime_delegation",
    "recommended_plugins",
    "external_codex_apps_open_page",
)


def internal_tag(text: str) -> str | None:
    match = re.match(r"<([a-z_]+)(?:\s|>)", text)
    return match.group(1) if match and match.group(1) in INTERNAL_TAGS else None


def audit() -> dict:
    cases = read(PRIVATE / "sample.json")
    excluded = []
    for case in cases:
        reasons = [
            f"{field}:{tag}"
            for field in ("input", "previous_input")
            if (tag := internal_tag(case[field]))
        ]
        if reasons:
            excluded.append({"id": case["id"], "arm": case["arm"], "reasons": reasons})
    result = {
        "sample_sha256": sha(PRIVATE / "sample.json"),
        "criterion": list(INTERNAL_TAGS),
        "selected": len(cases),
        "excluded": len(excluded),
        "eligible": len(cases) - len(excluded),
        "reasons": dict(Counter(r for c in excluded for r in c["reasons"])),
        "excluded_arms": dict(Counter(c["arm"] for c in excluded)),
        "excluded_cases": excluded,
    }
    write(PRIVATE / "extraction-audit.json", result)
    return result


if __name__ == "__main__":
    value = audit()
    print({k: v for k, v in value.items() if k != "excluded_cases"})
