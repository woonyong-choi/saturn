"""재개·분석 시 생성 요청을 원응답과 고정 정의에 대조한다."""

from __future__ import annotations

import hashlib
import json

from candidates import encode_request
from support import PRIVATE, read_json, read_rows, write_json


def seal_trials(cohort: str, trials: list[dict]) -> None:
    definitions = {}
    for trial in trials:
        definitions[trial["trial_id"]] = {
            "sha256": hashlib.sha256(encode_request(trial)).hexdigest(),
            "candidate_ids": trial["meta"]["candidate_ids"],
            "request_bytes": trial["meta"]["request_bytes"],
        }
    for row in read_rows(PRIVATE / "jev.jsonl"):
        if row["meta"]["cohort"] != cohort:
            continue
        definition = definitions[row["trial_id"].rsplit("-r", 1)[0]]
        body = json.dumps(row["request"], ensure_ascii=False).encode()
        if (
            hashlib.sha256(body).hexdigest() != definition["sha256"]
            or row["meta"]["candidate_ids"] != definition["candidate_ids"]
        ):
            raise RuntimeError("saved response and regenerated request differ")
    path = PRIVATE / f"trial-seal-{cohort}.json"
    if path.exists():
        if read_json(path) != definitions:
            raise RuntimeError("frozen trial definition changed")
    else:
        write_json(path, definitions)
