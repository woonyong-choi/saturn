"""공식 CLI의 모델 목록과 Claude 별칭의 실제 ID를 확인한다."""

from __future__ import annotations

import json
import selectors
import subprocess
from concurrent.futures import ThreadPoolExecutor
from typing import Any

from runtime import (
    PRIVATE,
    PUBLIC,
    call_cli,
    child_env,
    read,
    response_text,
    setup,
    write,
)


# cost: io 1 app-server and model/list pages; basis: estimate
def codex_models() -> list[dict]:
    target = PRIVATE / "codex-models.json"
    if target.exists():
        return read(target)
    proc = subprocess.Popen(
        ["codex", "app-server", "--stdio"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        cwd=PRIVATE / "runtime",
        env=child_env(),
    )
    selector = selectors.DefaultSelector()
    selector.register(proc.stdout, selectors.EVENT_READ)

    def request(identifier: str, method: str, params: dict) -> Any:
        proc.stdin.write(
            json.dumps(dict(id=identifier, method=method, params=params)) + "\n"
        )
        proc.stdin.flush()
        while selector.select(40):
            line = proc.stdout.readline()
            if not line:
                break
            row = json.loads(line)
            if row.get("id") == identifier:
                if "error" in row:
                    raise RuntimeError("app-server RPC failed")
                return row["result"]
        raise RuntimeError("app-server RPC timed out")

    try:
        request(
            1,
            "initialize",
            {
                "clientInfo": {"name": "saturn-experiment", "version": "1"},
                "capabilities": {"experimentalApi": True},
            },
        )
        proc.stdin.write(json.dumps({"method": "initialized", "params": {}}) + "\n")
        proc.stdin.flush()
        models, cursor = [], None
        for page in range(20):
            result = request(
                page + 2, "model/list", {"cursor": cursor} if cursor else {}
            )
            models.extend(result["data"])
            cursor = result.get("nextCursor")
            if not cursor:
                break
        write(target, models)
        return models
    finally:
        proc.terminate()
        proc.wait(timeout=10)
        selector.close()


def claude_model(alias: str) -> tuple[str, str]:
    record = call_cli(
        "claude", alias, 'Return exactly {"ready":true}.', "discovery-" + alias
    )
    _, envelope = response_text(record)
    models = list(envelope.get("modelUsage", {}))
    if len(models) != 1:
        raise RuntimeError("model identity not confirmed: " + alias)
    return alias, models[0]


if __name__ == "__main__":
    setup()
    models = codex_models()
    with ThreadPoolExecutor(max_workers=4) as pool:
        aliases = dict(pool.map(claude_model, ["default", "opus", "sonnet", "haiku"]))
    candidates = ["claude/" + a for a in aliases] + [
        "codex/" + m["model"] for m in models if not m.get("hidden", False)
    ]
    result = dict(
        candidates=candidates,
        claude_ids=aliases,
        codex_models=[
            {k: m.get(k) for k in ("model", "displayName", "description", "isDefault")}
            for m in models
        ],
    )
    write(PUBLIC / "models.json", result)
    print(json.dumps(result, ensure_ascii=False))
