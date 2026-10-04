"""지정한 한 라벨 통로를 중단 지점부터 실행한다."""

from __future__ import annotations

import argparse

from labels import run_lane
from storage import MODELS, PRIVATE, ensure_private, read_json


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("lane", choices=(*MODELS, "adjudicated"))
    args = parser.parse_args()
    ensure_private()
    selection = read_json(PRIVATE / "selection.json")
    for meta in sorted(
        selection["selected"],
        key=lambda row: (row["user_turns"], row["conversation_id"]),
    ):
        conversation = read_json(
            PRIVATE / "conversations" / (meta["conversation_id"] + ".json")
        )
        run_lane(conversation, args.lane)


if __name__ == "__main__":
    main()
