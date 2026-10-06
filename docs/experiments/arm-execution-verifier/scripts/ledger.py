"""보존 우선 비교(#540) 수집 장부: provider를 부르기 전에 시도를 남기고, 봉인된 호출 상한을 재시작을 넘어 지킨다.

장부는 추가만 하는 JSON Lines 한 파일이다. 모든 줄은 fsync로 디스크에 닿은 뒤에야 다음 단계로 간다.
- start: provider를 처음 부르기 전에 쓴다. 예약 호출 수(보수적 상한)와 source 스냅샷 시도를 담는다.
- end: 결과를 안 뒤에 쓴다. 실제 호출 수를 담으며, 세지 못한 값은 null(알 수 없음)이다. 0으로 바꾸지 않는다.
start가 있고 end가 없는 시도는 중단된 시도다. 전송 여부를 모르므로 자동으로 다시 보내지 않고, 예약 수를 그대로 쓴 것으로 센다.
"""

from __future__ import annotations

import fcntl
import json
import os
import stat
from pathlib import Path

FIELDS = ("inputs", "jev")


class BudgetStop(RuntimeError):
    """남은 허용량이 보수적으로 잡은 시도 하나를 덮지 못한다."""


class Ledger:
    def __init__(self, folder: Path, caps: dict):
        if folder.is_symlink():
            raise RuntimeError("ledger folder must not be a symlink: " + str(folder))
        folder.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.caps = dict(inputs=caps["provider_inputs"], jev=caps["jev_calls"])
        self.path = folder / "ledger.jsonl"
        self.dir_fd = os.open(folder, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        os.fchmod(self.dir_fd, 0o700)
        self.fd = os.open(self.path, os.O_RDWR | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW, 0o600)
        if not stat.S_ISREG(os.fstat(self.fd).st_mode):
            os.close(self.fd)
            os.close(self.dir_fd)
            raise RuntimeError("ledger path is not a regular file: " + str(self.path))
        os.fchmod(self.fd, 0o600)
        try:
            fcntl.flock(self.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            os.close(self.fd)
            os.close(self.dir_fd)
            raise RuntimeError("another collector holds the ledger: " + str(self.path)) from error
        os.fsync(self.dir_fd)
        self.starts: dict = {}
        self.ends: dict = {}
        with os.fdopen(os.dup(self.fd), "r", encoding="utf-8") as reader:
            reader.seek(0)
            lines = reader.read().splitlines()
        for line in lines:
            row = json.loads(line)
            (self.starts if row["op"] == "start" else self.ends)[row["id"]] = row

    def close(self) -> None:
        os.close(self.fd)
        os.close(self.dir_fd)

    def _append(self, row: dict) -> None:
        data = (json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n").encode()
        while data:
            written = os.write(self.fd, data)
            if written == 0:
                raise OSError("ledger write made no progress")
            data = data[written:]
        os.fsync(self.fd)

    def attempted(self, attempt_id: str) -> bool:
        return attempt_id in self.starts

    def status(self, attempt_id: str) -> str | None:
        if attempt_id not in self.starts:
            return None
        return self.ends[attempt_id]["status"] if attempt_id in self.ends else "attempted_unfinished"

    def committed(self) -> dict:
        """상한에서 이미 쓴 것으로 치는 수. 끝난 시도는 실제 수, 중단됐거나 세지 못한 값은 예약 수."""
        out = dict.fromkeys(FIELDS, 0)
        for attempt_id, start in self.starts.items():
            actual = self.ends.get(attempt_id, {}).get("actual", {})
            for field in FIELDS:
                value = actual.get(field)
                out[field] += start["reserve"][field] if value is None else value
        return out

    def remaining(self) -> dict:
        used = self.committed()
        return {f: self.caps[f] - used[f] for f in FIELDS}

    def start(self, attempt_id: str, kind: str, reserve: dict, **extra) -> None:
        """예약이 남은 허용량 안일 때만 시도를 기록한다. 같은 시도를 두 번 시작하지 않는다. 이 호출이 돌아온 뒤에만 provider를 부른다."""
        if attempt_id in self.starts:
            raise RuntimeError("attempt already started: " + attempt_id)
        left = self.remaining()
        short = {f: reserve[f] - left[f] for f in FIELDS if reserve[f] > left[f]}
        if short:
            raise BudgetStop(f"{attempt_id} needs {reserve} but the sealed allowance has {left} left")
        row = dict(op="start", id=attempt_id, kind=kind, reserve=dict(reserve), **extra)
        self._append(row)
        self.starts[attempt_id] = row

    def end(self, attempt_id: str, status: str, actual: dict | None = None, **extra) -> None:
        if attempt_id not in self.starts or attempt_id in self.ends:
            raise RuntimeError("attempt is missing or already ended: " + attempt_id)
        actual = {f: (actual or {}).get(f) for f in FIELDS}
        row = dict(op="end", id=attempt_id, status=status, actual=actual, **extra)
        self._append(row)
        self.ends[attempt_id] = row
        exceeded = {f: actual[f] for f in FIELDS
                    if actual[f] is not None and actual[f] > self.starts[attempt_id]["reserve"][f]}
        if exceeded or any(self.committed()[f] > self.caps[f] for f in FIELDS):
            raise BudgetStop(f"actual calls exceeded the reserved bound: {attempt_id} {exceeded}")

    def report(self) -> dict:
        """예약과 실제를 함께 보인다. 세지 못한 값은 합에 넣지 않고 개수로 따로 센다."""
        out = dict(caps=self.caps, committed=self.committed(), remaining=self.remaining(),
                   attempts=len(self.starts), unfinished=sum(i not in self.ends for i in self.starts))
        for field in FIELDS:
            known = [e["actual"][field] for e in self.ends.values() if e["actual"][field] is not None]
            out[field] = dict(reserved=sum(s["reserve"][field] for s in self.starts.values()), actual_known=sum(known),
                              actual_unknown_attempts=len(self.starts) - len(known))
        return out
