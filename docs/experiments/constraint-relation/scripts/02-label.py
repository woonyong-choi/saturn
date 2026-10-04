"""짧은 원대화를 독립된 두 통로와 별도 판정으로 확장한다."""

from __future__ import annotations

import argparse
import json
import time

from support import (
    PRIVATE,
    SOURCE,
    initialize,
    labeler,
    read_json,
    read_rows,
    write_json,
)


def make_batch(ids: list[str], lane: str, index: int) -> tuple[str, dict]:
    prompts, properties = [], {}
    for cid in ids:
        c = read_json(SOURCE / "conversations" / (cid + ".json"))
        prompts.append(labeler.build_prompt(c, c["turns"], []))
        properties[cid] = labeler._label_schema(
            [t["turn_id"] for t in c["turns"]], set()
        )
    prompt = (
        "각 conversation_id는 서로 독립된 대화다. 대화 사이 제약을 전달하지 않는다. 최상위 키를 conversation_id로 하는 JSON 객체를 답한다.\n"
        + "\n".join(prompts)
    )
    if lane == "adjudicated":
        first = [
            read_json(PRIVATE / "extension" / f"{model}-{index}.json")
            for model in ("gpt-6-astra", "gpt-5.6-luna")
        ]
        prompt += (
            "\n별도 판정자로서 두 독립 라벨의 근거를 재검토한다. 다수결 대신 원문으로 판정하고 불명확하면 ambiguous=true다. Jev 출력은 보지 않는다.\n"
            + json.dumps(first, ensure_ascii=False)
        )
    return prompt, {
        "type": "object",
        "additionalProperties": False,
        "required": ids,
        "properties": properties,
    }


def run_batch(ids: list[str], lane: str, index: int) -> None:
    output = PRIVATE / "extension" / f"{lane}-{index}.json"
    if output.exists():
        return
    if lane == "adjudicated":
        inputs = [
            PRIVATE / "extension" / f"{model}-{index}.json"
            for model in ("gpt-6-astra", "gpt-5.6-luna")
        ]
        while not all(path.exists() for path in inputs):
            time.sleep(2)
        if any(read_json(path).get("invalid_batch") for path in inputs):
            write_json(output, {"invalid_batch": True})
            return
    prompt, schema = make_batch(ids, lane, index)
    model = "gpt-6-astra" if lane == "adjudicated" else lane
    for attempt in range(2):
        trial = f"extension-{lane}-{index}-{attempt}"
        receipt = PRIVATE / "codex" / trial / "receipt.json"
        value = {}
        try:
            if receipt.exists():
                record = read_json(receipt)
                if record["returncode"] != 0:
                    raise RuntimeError("label model failed")
                text = record["stdout"]
                value = json.loads(text[text.find("{") : text.rfind("}") + 1])
            else:
                reserved = {r["trial_id"] for r in read_rows(PRIVATE / "calls.jsonl")}
                if trial in reserved:
                    raise RuntimeError(
                        "reserved label call has no receipt; do not resend"
                    )
                value = labeler.call_codex(model, prompt, trial, schema)
            for cid in ids:
                c = read_json(SOURCE / "conversations" / (cid + ".json"))
                labeler.validate_labels(
                    value[cid], [t["turn_id"] for t in c["turns"]], set()
                )
            write_json(output, value)
            print({"lane": lane, "batch": index, "status": "ok"}, flush=True)
            return
        except (KeyError, ValueError, TypeError) as error:
            prompt += (
                "\n형식 오류를 고쳐 전체 객체를 다시 답한다: "
                + str(error)
                + "\n"
                + json.dumps(value, ensure_ascii=False)
            )
    write_json(output, {"invalid_batch": True})


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("lane", choices=("gpt-6-astra", "gpt-5.6-luna", "adjudicated"))
    args = parser.parse_args()
    initialize()
    for index, ids in enumerate(read_json(PRIVATE / "plan.json")["label_batches"]):
        run_batch(ids, args.lane, index)


if __name__ == "__main__":
    main()
