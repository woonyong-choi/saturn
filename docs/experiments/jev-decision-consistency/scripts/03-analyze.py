"""data/processed/ 표로 개발 요약, 확인 가설 판정, 기술 통계를 results/에 만든다.

사용: python3 scripts/03-analyze.py [--check]
같은 입력이면 같은 바이트를 낸다. --check는 파일을 쓰지 않고 저장된 결과와 바이트를 비교한다.
"""
import csv
import io
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import plan  # noqa: E402
from stats import cluster_bootstrap, rnd, wilson  # noqa: E402

ROOT = plan.ROOT
PROCESSED = ROOT / "data" / "processed"
RESULTS = ROOT / "results"
VARIANT_GROUPS = ("neutral_insert", "injection_insert", "paraphrase", "flip")
ROLES = ("is_constraint", "replaces_1")
RANGE_LIMIT = 0.10


def read_csv(name: str) -> list[dict]:
    with (PROCESSED / name).open(encoding="utf-8", newline="") as file:
        return list(csv.DictReader(file))


def number(value: str) -> float | None:
    return float(value) if value != "" else None


class Data:
    def __init__(self) -> None:
        self.trials = read_csv("trials.csv")
        self.legacy = read_csv("legacy_152.csv")
        self.inputs: dict[str, dict] = {}
        self.pairs: dict[str, dict] = {}
        for item in plan.load()[0]:
            self.inputs[item["id"]] = item
        for item in plan.load()[1]:
            self.pairs[item["id"]] = item
        self.edits = plan.variants()

    def item(self, role: str, item_id: str) -> dict:
        return self.inputs[item_id] if role == "is_constraint" else self.pairs[item_id]

    def cluster(self, role: str, item_id: str) -> str:
        item = self.item(role, item_id)
        return item["previous"] if role == "is_constraint" else item["earlier"]["text"]

    def select(self, phase: str, group: str, role: str) -> dict[str, list[dict]]:
        out: dict[str, list[dict]] = {}
        for t in self.trials:
            if t["phase"] == phase and t["group"] == group and t["role"] == role:
                out.setdefault(t["item_id"], []).append(t)
        for rows in out.values():
            rows.sort(key=lambda r: int(r["rep"]))
        return dict(sorted(out.items()))


def predicted(role: str, band: str) -> bool | str:
    """행동 구간을 라벨과 맞대는 이진 판정. is_constraint는 0.7 이상, replaces_1은 대체 구간."""
    if band == "":
        return ""
    return band in ("auto", "ask") if role == "is_constraint" else band == "replace"


def correct(role: str, item: dict, band: str) -> bool | None:
    if band == "":
        return None
    if role == "is_constraint":
        return predicted(role, band) is item["label"]
    if item["label"] == "replaces":
        return band == "replace"
    if item["label"] == "compatible":
        return band == "none"
    return band == "possible"


def baseline_band(rows: list[dict]) -> str:
    counts = Counter(r["band"] for r in rows if r["status"] == "ok")
    if not counts:
        return "tie"
    ranked = counts.most_common()
    return "tie" if len(ranked) > 1 and ranked[0][1] == ranked[1][1] else ranked[0][0]


def majority_band(rows: list[dict]) -> str:
    """3회 중 같은 구간이 2회 이상이면 그 구간, 아니면 빈 문자열."""
    counts = Counter(r["band"] for r in rows if r["status"] == "ok")
    if not counts:
        return ""
    band, n = counts.most_common(1)[0]
    return band if n >= 2 and sum(1 for c in counts.values() if c == n) == 1 else ""


def proportion(name: str, flags: list[tuple[str, int]], threshold: float, cluster: bool, excluded: int = 0) -> dict:
    n = len(flags)
    k = sum(f for _, f in flags)
    low, high = wilson(k, n)
    boot_low, boot_high = cluster_bootstrap(flags) if cluster else (None, None)
    lowers = [v for v in (low, boot_low) if v is not None]
    if n == 0 or len(lowers) < (2 if cluster else 1):
        verdict = "hold"
    elif all(v > threshold for v in lowers):
        verdict = "adopt"
    elif high is not None and high < threshold:
        verdict = "reject"
    else:
        verdict = "hold"
    return {"hypothesis": name, "k": k, "n": n, "excluded": excluded, "value": rnd(k / n if n else None),
            "wilson_low": rnd(low), "wilson_high": rnd(high), "cluster_low": rnd(boot_low), "cluster_high": rnd(boot_high),
            "threshold": threshold, "verdict": verdict}


def identical_stats(data: Data, phase: str, role: str) -> dict:
    items = data.select(phase, "identical", role)
    agree, rng, exact, base, rows = [], [], [], {}, []
    for item_id, reps in items.items():
        ok = [r for r in reps if r["status"] == "ok"]
        complete = len(ok) == len(reps)
        bands = {r["band"] for r in ok}
        answers = [float(r["answer"]) for r in ok]
        a = int(complete and len(bands) == 1)
        r_ok = int(complete and max(answers) - min(answers) <= RANGE_LIMIT + 1e-9)
        agree.append((data.cluster(role, item_id), a))
        rng.append((data.cluster(role, item_id), r_ok))
        exact.append(int(complete and len(set(answers)) == 1))
        base[item_id] = baseline_band(reps)
        first3 = [r for r in reps if int(r["rep"]) <= 3]
        rows.append({"item_id": item_id, "agree5": a, "agree3": int(len([r for r in first3 if r["status"] == "ok"]) == len(first3) and len({r["band"] for r in first3}) == 1),
                     "range_ok": r_ok, "complete": int(complete)})
    return {"items": items, "agree": agree, "range": rng, "exact": exact, "baseline": base, "rows": rows}


def accuracy_stats(data: Data, role: str, stats: dict) -> dict:
    ok = wrong = 0
    bands: Counter = Counter()
    by_category: dict[str, list[int]] = {}
    for item_id, band in stats["baseline"].items():
        if band == "tie":
            continue
        item = data.item(role, item_id)
        bands[band] += 1
        c = correct(role, item, band)
        by_category.setdefault(item["category"], []).append(int(bool(c)))
        ok += int(bool(c))
        wrong += int(not c)
    return {"n": ok + wrong, "correct": ok, "wrong": wrong, "bands": dict(sorted(bands.items())),
            "by_category": {k: {"k": sum(v), "n": len(v)} for k, v in sorted(by_category.items())}}


def dev_summary(data: Data) -> dict:
    out: dict = {}
    for role in ROLES:
        stats = identical_stats(data, "dev", role)
        n = len(stats["rows"])
        out[role] = {
            "items": n,
            "agree5": sum(r["agree5"] for r in stats["rows"]), "agree3": sum(r["agree3"] for r in stats["rows"]),
            "range_ok": sum(r["range_ok"] for r in stats["rows"]), "exact_repeat": sum(stats["exact"]),
            "agree3_equals_agree5": all(r["agree3"] == r["agree5"] for r in stats["rows"]),
            "incomplete_items": sum(1 - r["complete"] for r in stats["rows"]),
            "accuracy": accuracy_stats(data, role, stats),
        }
    items = data.select("dev", "identical", "relation_choice")
    same = same3 = complete_n = 0
    for reps in items.values():
        ok = [r for r in reps if r["status"] == "ok"]
        complete_n += int(len(ok) == len(reps))
        same += int(len(ok) == len(reps) and len({r["top_option"] for r in ok}) == 1)
        first3 = [r for r in reps if int(r["rep"]) <= 3]
        same3 += int(all(r["status"] == "ok" for r in first3) and len({r["top_option"] for r in first3}) == 1)
    out["relation_choice"] = {"items": len(items), "top_same5": same, "top_same3": same3, "complete_items": complete_n}
    return out


def variant_stats(data: Data, role: str, group: str, base: dict[str, str]) -> tuple[list[tuple[str, int]], int]:
    flags, excluded = [], 0
    items = data.select("confirm", group, role)
    for item_id, reps in items.items():
        if base.get(item_id, "tie") == "tie":
            excluded += 1
            continue
        stable = len(reps) == 3 and all(r["status"] == "ok" and r["band"] == base[item_id] for r in reps)
        flags.append((data.cluster(role, item_id), int(stable)))
    return flags, excluded


def flip_stats(data: Data, role: str, base: dict[str, str]) -> tuple[list[tuple[str, int]], int]:
    flags, excluded = [], 0
    for item_id, reps in data.select("confirm", "flip", role).items():
        item = data.item(role, item_id)
        if not correct(role, item, base.get(item_id, "tie")) or base.get(item_id, "tie") == "tie":
            excluded += 1
            continue
        new_label = data.edits[item_id]["flip_label"]
        band = majority_band(reps)
        if role == "is_constraint":
            hit = band != "" and predicted(role, band) is new_label
        else:
            hit = band == ("replace" if new_label == "replaces" else "none")
        flags.append((data.cluster(role, item_id), int(hit)))
    return flags, excluded


def time_layer(data: Data, role: str, base: dict[str, str]) -> tuple[list[tuple[str, int]], int, dict]:
    from importlib import import_module
    band = import_module("02-process").band
    old = {r["item_id"]: r for r in data.legacy if r["role"] == role}
    flags, excluded, deltas, models = [], 0, [], Counter()
    items = data.select("confirm", "identical", role)
    for item_id, reps in items.items():
        r = old.get(item_id)
        if r is None or r["status"] != "ok" or r["answer"] == "":
            excluded += 1
            continue
        models[r["model"]] += 1
        old_band = band(role, float(r["answer"]))
        flags.append((data.cluster(role, item_id), int(base[item_id] == old_band)))
        answers = [float(x["answer"]) for x in reps if x["status"] == "ok"]
        if answers:
            deltas.append(abs(sorted(answers)[len(answers) // 2] - float(r["answer"])))
    deltas.sort()
    quant = {"median": rnd(deltas[len(deltas) // 2]) if deltas else None, "p95": rnd(deltas[int(0.95 * (len(deltas) - 1))]) if deltas else None,
             "max": rnd(deltas[-1]) if deltas else None, "old_models": dict(sorted(models.items()))}
    return flags, excluded, quant


def choice_stats(data: Data) -> tuple[list[tuple[str, int]], dict]:
    flags, calibration = [], {"low": [0, 0], "high": [0, 0]}
    items = data.select("confirm", "order", "relation_choice")
    gap = 0
    for item_id, reps in items.items():
        ok = [r for r in reps if r["status"] == "ok"]
        same = len(ok) == len(reps) == 3 and len({r["top_option"] for r in ok}) == 1
        flags.append((data.pairs[item_id]["earlier"]["text"], int(same)))
        want = plan.CHOICE_LABEL[data.pairs[item_id]["label"]]
        for r in ok:
            bucket = "high" if float(r["confidence"]) >= 0.6 else "low"
            calibration[bucket][1] += 1
            calibration[bucket][0] += int(r["top_option"] == want)
            if r["api_confidence"] != "" and abs(float(r["api_confidence"]) - float(r["confidence"])) > 0.01:
                gap += 1
    return flags, {"correct_by_confidence": {k: {"k": v[0], "n": v[1]} for k, v in calibration.items()}, "api_confidence_mismatch": gap}


def call_stats(data: Data) -> dict:
    out: dict = {}
    for phase in ("dev", "confirm"):
        rows = [t for t in data.trials if t["phase"] == phase]
        out[phase] = {
            "trials": len(rows), "attempts": sum(int(t["attempts"]) for t in rows),
            "status": dict(sorted(Counter(t["status"] for t in rows).items())),
            "models": dict(sorted(Counter(t["model"] for t in rows).items())),
            "usage_missing": sum(1 for t in rows if t["input_tokens"] == ""),
            "input_tokens": sum(int(t["input_tokens"]) for t in rows if t["input_tokens"] != ""),
            "output_tokens": sum(int(t["output_tokens"]) for t in rows if t["output_tokens"] != ""),
        }
    return out


def confirm_summary(data: Data) -> dict:
    hyps: list[dict] = []
    detail: dict = {}
    order = {"is_constraint": ["H1", "H3", "H5", "H7", "H9", "H11", "H13"], "replaces_1": ["H2", "H4", "H6", "H8", "H10", "H12", "H14"]}
    for role in ROLES:
        names = dict(zip(("agree", "range", "neutral", "injection", "paraphrase", "flip", "time"), order[role]))
        stats = identical_stats(data, "confirm", role)
        hyps.append(proportion(names["agree"], stats["agree"], 0.90, True))
        hyps.append(proportion(names["range"], stats["range"], 0.90, True))
        base = stats["baseline"]
        for key, group in (("neutral", "neutral_insert"), ("injection", "injection_insert"), ("paraphrase", "paraphrase")):
            flags, excluded = variant_stats(data, role, group, base)
            hyps.append(proportion(names[key], flags, 0.80, False, excluded))
        flags, excluded = flip_stats(data, role, base)
        hyps.append(proportion(names["flip"], flags, 0.80, False, excluded))
        flags, excluded, quant = time_layer(data, role, base)
        hyps.append(proportion(names["time"], flags, 0.90, True, excluded))
        detail[role] = {
            "items": len(stats["rows"]), "exact_repeat": sum(stats["exact"]),
            "incomplete_items": sum(1 - r["complete"] for r in stats["rows"]),
            "baseline_tie": sum(1 for b in base.values() if b == "tie"),
            "accuracy": accuracy_stats(data, role, stats), "time_delta": quant,
        }
    flags, calibration = choice_stats(data)
    hyps.append(proportion("H15", flags, 0.85, True))
    detail["relation_choice"] = calibration
    hyps.sort(key=lambda h: int(h["hypothesis"][1:]))
    verdicts = {}
    for role, names in (("is_constraint", order["is_constraint"]), ("replaces_1", order["replaces_1"])):
        vs = [h["verdict"] for h in hyps if h["hypothesis"] in names]
        verdicts[role] = "reject" if "reject" in vs else "adopt" if all(v == "adopt" for v in vs) else "hold"
    verdicts["relation_choice"] = "exploratory"
    return {"hypotheses": hyps, "role_verdicts": verdicts, "detail": detail}


def to_csv(rows: list[dict]) -> str:
    buffer = io.StringIO(newline="")
    writer = csv.DictWriter(buffer, fieldnames=list(rows[0]), lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows)
    return buffer.getvalue()


def outputs() -> dict[str, str]:
    data = Data()
    summary: dict = {"calls": call_stats(data), "call_cap": plan.CALL_CAP, "dev": dev_summary(data)}
    files: dict[str, str] = {}
    if any(t["phase"] == "confirm" for t in data.trials):
        summary["confirm"] = confirm_summary(data)
        files["tables/hypotheses.csv"] = to_csv(summary["confirm"]["hypotheses"])
    files["summary.json"] = json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    return files


def main() -> int:
    files = outputs()
    if "--check" in sys.argv:
        bad = [n for n, text in files.items() if (RESULTS / n).read_text(encoding="utf-8") != text]
        if bad:
            print(f"분석 결과가 저장된 파일과 다르다: {', '.join(bad)}", file=sys.stderr)
            return 1
        print("분석 결과 바이트 일치")
        return 0
    for name, text in files.items():
        path = RESULTS / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline="")
    print(f"분석 끝: {', '.join(files)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
