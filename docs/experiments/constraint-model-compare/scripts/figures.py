"""공개 집계에서 차트 입력을 만들고 같은 도구로 정지 SVG를 생성한다."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

from runtime import PUBLIC, ROOT, read, write


def main() -> None:
    summary = read(PUBLIC / "results/summary.json")
    data: dict[str, list] = {"precision_recall": [], "f1": [], "omitted": []}
    for row in summary["jev_curve"]:
        for group, keys in (
            ("precision_recall", ("precision", "recall")),
            ("f1", ("f1",)),
        ):
            if any(row[key]["value"] is None or not row[key]["ci95"] for key in keys):
                data["omitted"].append({"group": group, "threshold": row["threshold"]})
                continue
            point = {"x": row["threshold"]}
            for key in keys:
                point[key] = row[key]["value"] * 100
                point[key + ".low"] = row[key]["ci95"][0] * 100
                point[key + ".high"] = row[key]["ci95"][1] * 100
            data[group].append(point)
    write(PUBLIC / "results/chart.json", data)
    cli = Path(
        os.environ.get(
            "MUTOSCOPE_CLI",
            str(Path.home() / "workspace/oss/mutoscope/src/cli.js"),
        )
    )
    for name in ("precision-recall", "f1"):
        source = ROOT / f"docs/assets/constraint-model-compare-{name}.muto"
        for operation in ("check", "render"):
            command = [
                "node",
                str(cli),
                operation,
                str(source),
                "--strict",
                "--require-data",
                "--require-ci",
            ]
            if operation == "render":
                command.append("--static")
            subprocess.run(command, cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
