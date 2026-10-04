"""가림 함수가 설계의 비밀값·이메일·절대 경로 규칙을 지키는지 확인한다."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import EMAIL_TOKEN, MASK_TOKEN, PATH_TOKEN, mask_text  # noqa: E402


def main() -> int:
    cases = [
        ("key sk-ant-api-1234567890123456 and Bearer abcdefghijklmnop", MASK_TOKEN),
        ("mail woonyong@example.com", EMAIL_TOKEN),
        ("file /Users/woonyong/workspace/oss/saturn/src/lib.rs", f"{PATH_TOKEN}/lib.rs"),
    ]
    for source, expected in cases:
        result = mask_text(source)
        if expected not in result or any(secret in result for secret in ("sk-ant-api-", "woonyong@example.com", "/Users/woonyong")):
            raise AssertionError(f"가림 실패: {source!r} -> {result!r}")
    print(f"가림 검사 통과: {len(cases)}건")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
