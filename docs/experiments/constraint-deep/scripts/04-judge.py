"""등록·질문 문장 비교·관계 판단을 저장하며 수집한다."""

from __future__ import annotations

import argparse
import os

from judge import (
    collect_alternatives,
    collect_relations,
    collect_routes,
    load_conversations,
)
from storage import ensure_private


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("phase", choices=("route", "alternative", "relation", "all"))
    args = parser.parse_args()
    ensure_private()
    if not os.environ.get("SATURN_JUDGE_KEY"):
        raise RuntimeError("SATURN_JUDGE_KEY missing")
    conversations = load_conversations()
    for phase, collect in [
        ("route", collect_routes),
        ("alternative", collect_alternatives),
        ("relation", collect_relations),
    ]:
        if args.phase in (phase, "all"):
            collect(conversations)


if __name__ == "__main__":
    main()
