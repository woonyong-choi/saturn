"""첫 반복을 분석하고 표와 보고서의 유일한 수치 원본을 만든다."""

from __future__ import annotations

import csv
from collections import Counter, defaultdict

from common import PRIVATE, PUBLIC, read_json, read_rows, write_json
from metrics import aggregate, bootstrap, decision, holm, paired, quantile, ratio, tail


# cost: io 1 CSV file; basis: estimate
def csv_table(name: str, rows: list[dict]) -> None:
    path = PUBLIC / "results/tables" / name
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


# cost: time O(bn), heap O(b+n), io local tables; vars: b = bootstrap repetitions, n = observations; basis: estimate
def main() -> None:
    rows = read_rows(PRIVATE / "processed.jsonl")
    first = [r for r in rows if r["repeat"] == 1]
    curves = []
    conditions = []
    all_metrics = {}
    for form in ("pair", "choice"):
        for record in (False, True):
            selected = [r for r in first if r["form"] == form and r["record"] == record]
            key = f"{form}-{int(record)}"
            for t in [i / 100 for i in range(50, 100, 5)]:
                m = aggregate(selected, t)
                all_metrics[f"{key}-{t:.2f}"] = m
                curves.append(
                    dict(
                        form=form,
                        record=int(record),
                        threshold=t,
                        **{metric: m[metric]["value"] for metric in m},
                    )
                )
            groups = [("all", "all", selected)]
            for field in ("n", "category", "source", "intent"):
                for value in sorted({r[field] for r in selected}):
                    groups.append(
                        (field, str(value), [r for r in selected if r[field] == value])
                    )
            for n, category in sorted({(r["n"], r["category"]) for r in selected}):
                groups.append(
                    (
                        "n_category",
                        f"{n}/{category}",
                        [
                            r
                            for r in selected
                            if r["n"] == n and r["category"] == category
                        ],
                    )
                )
            for field, value, group in groups:
                m = aggregate(group, 0.8)
                conditions.append(
                    dict(
                        form=form,
                        record=int(record),
                        field=field,
                        value=value,
                        n=len(group),
                        metrics=m,
                    )
                )
    base = [r for r in first if r["form"] == "pair" and r["record"]]
    newest = [
        r for r in base if r["category"] in ("direct", "deictic", "indirect", "mixed")
    ]
    content = [r for r in base if r["category"] == "content"]
    h2_conditions = []
    for form in ("pair", "choice"):
        for record in (False, True):
            for group_name, categories in [
                ("recent", ("direct", "deictic", "indirect", "mixed")),
                ("content", ("content",)),
            ]:
                for n in (1, 3, 10):
                    group = [
                        r
                        for r in first
                        if r["form"] == form
                        and r["record"] == record
                        and r["category"] in categories
                        and r["n"] == n
                    ]
                    h2_conditions.append(
                        dict(
                            form=form,
                            record=record,
                            reference=group_name,
                            n=n,
                            metrics=aggregate(group, 0.8),
                        )
                    )
    hypotheses = {}
    ps = {}
    for key, selected, metric, target, upper in [
        ("H1_precision", base, "precision", 0.9, True),
        ("H1_recall", base, "recall", 0.85, True),
        ("H2_recent", newest, "target_accuracy", 0.95, True),
        ("H2_content", content, "target_accuracy", 0.85, True),
        ("H3", base, "false_release", 0.02, False),
    ]:
        measure = aggregate(selected, 0.8)[metric]
        boot = bootstrap(selected, metric, 0.8)
        p = tail(measure["k"], measure["n"], target, upper=upper)
        ps[key] = p
        hypotheses[key] = dict(
            metric=metric, **measure, bootstrap=boot, p=p, target=target, upper=upper
        )
    for key, left, right in [
        ("H4", [r for r in first if r["form"] == "pair" and not r["record"]], base),
        ("H5", base, [r for r in first if r["form"] == "choice" and r["record"]]),
    ]:
        hypotheses[key] = paired(left, right, 0.8)
        ps[key] = hypotheses[key]["p"]
    adjusted = holm(ps)
    for key, h in hypotheses.items():
        h["adjusted_p"] = adjusted[key]
        if key in ("H4", "H5"):
            accept = (
                h["low"] is not None
                and h["low"] > 0
                and h["cluster_low"] > 0
                and adjusted[key] < 0.05
                and (key == "H4" or h["byte_high"] < 0)
            )
            reject = h["high"] is not None and (
                h["high"] < 0 or (key == "H5" and h["byte_low"] > 0)
            )
        else:
            upper = h["upper"]
            boundary = h["target"]
            value = h["value"]
            boot = h["bootstrap"]
            accept = (
                value is not None
                and adjusted[key] < 0.05
                and (
                    value >= boundary
                    and boot["low"] is not None
                    and boot["low"] >= boundary
                    if upper
                    else value <= boundary
                    and boot["high"] is not None
                    and boot["high"] <= boundary
                )
            )
            reject = value is not None and (
                h["high"] < boundary if upper else h["low"] > boundary
            )
        h["verdict"] = "채택" if accept else "기각" if reject else "보류"
    repeats = []
    grouped = defaultdict(list)
    for r in rows:
        grouped[(r["condition"], r["item_id"])].append(r)
    for cond in sorted({r["condition"] for r in rows}):
        k = 0
        valid = 0
        n = 0
        for (c, _), group in grouped.items():
            if c != cond:
                continue
            n += 1
            if len(group) == 3 and all(r["valid"] for r in group):
                valid += 1
                ds = [decision(r, 0.8) for r in group]
                k += (
                    len({(d["request"], d["target"], tuple(d["actions"])) for d in ds})
                    == 1
                )
        repeats.append(
            dict(
                condition=cond,
                valid=valid,
                total=n,
                agreement=ratio(k, n),
                conditional_agreement=ratio(k, valid),
            )
        )
    sizes = []
    for form in ("pair", "choice"):
        for record in (False, True):
            xs = [
                r["request_bytes"]
                for r in first
                if r["form"] == form and r["record"] == record
            ]
            sizes.append(
                dict(
                    form=form,
                    record=record,
                    min=min(xs) if xs else None,
                    median=quantile(xs, 0.5),
                    p95=quantile(xs, 0.95),
                    max=max(xs) if xs else None,
                )
            )
    candidates = []
    for form in ("pair", "choice"):
        for t in [i / 100 for i in range(50, 100, 5)]:
            m = all_metrics[f"{form}-1-{t:.2f}"]
            if (
                m["precision"]["low"] is not None
                and m["precision"]["low"] >= 0.9
                and m["recall"]["low"] >= 0.85
                and m["false_release"]["high"] is not None
                and m["false_release"]["high"] <= 0.02
            ):
                candidates.append(dict(form=form, threshold=t))
                break
    calls = read_rows(PRIVATE / "calls.jsonl")
    raw = read_rows(PRIVATE / "jev.jsonl")
    exclusions = Counter()
    for item in read_json(PRIVATE / "excluded.json"):
        if item.get("exclusion"):
            exclusions[item["exclusion"]] += 1
        elif item["source"] == "real":
            exclusions["real_not_agreed_none"] += 1
        elif item["label"]["ambiguous"]:
            exclusions["synthetic_ambiguous"] += 1
        elif (item["intent"], item["target"]) != (
            item["label"]["intent"],
            item["label"]["target"],
        ):
            exclusions["synthetic_disagreement"] += 1
        else:
            exclusions["synthetic_duplicate"] += 1
    real_labels = []
    for path in sorted((PRIVATE / "codex").glob("real-astra-*/receipt.json")):
        receipt = read_json(path)
        text = receipt.get("stdout", "")
        if receipt.get("returncode") == 0:
            import json

            real_labels.extend(
                json.loads(text[text.find("{") : text.rfind("}") + 1])["rows"]
            )
    real_screened = len(real_labels)
    real_unused = (
        real_screened
        - exclusions.get("real_not_agreed_none", 0)
        - read_json(PRIVATE / "flow.json")["real_retained"]
    )
    agreement_n = read_json(PRIVATE / "flow.json")["synthetic_planned"]
    agreement_k = read_json(PRIVATE / "flow.json")[
        "synthetic_retained"
    ] + exclusions.get("synthetic_duplicate", 0)
    summary = dict(
        label_flow=dict(
            real_screened=real_screened,
            real_unused=real_unused,
            synthetic_first_agreement=ratio(agreement_k, agreement_n),
        ),
        codex_phases=dict(
            Counter(
                "generate"
                if c["trial_id"].startswith("generate-")
                else "independent_label"
                if c["trial_id"].startswith("label-")
                else "real_label"
                if c["trial_id"].startswith("real-")
                else "review"
                for c in calls
                if c["kind"] == "codex"
            )
        ),
        exclusions=dict(exclusions),
        run=read_json(PRIVATE / "run.json"),
        flow=read_json(PRIVATE / "flow.json"),
        sample=dict(
            items=len(first) // 4,
            positive=sum(r["intent"] != "none" for r in base),
            negative=sum(r["intent"] == "none" for r in base),
            clusters=len({r["cluster"] for r in base}),
            rules=len(read_json(PRIVATE / "plan.json")["rules"]),
            public_redactions=sum(
                r["text_redacted"] for r in read_json(PUBLIC / "data/generated.json")
            ),
        ),
        calls=dict(Counter(c["kind"] for c in calls)),
        statuses=dict(Counter(r["status"] for r in rows)),
        http_statuses=dict(Counter(str(r.get("http_status")) for r in raw)),
        models=dict(Counter(str(r.get("model")) for r in raw)),
        hypotheses=hypotheses,
        h2_conditions=h2_conditions,
        curves=all_metrics,
        conditions=conditions,
        repeats=repeats,
        sizes=sizes,
        recommendation=candidates,
        power=[
            dict(n=30, p0=0.85, p1=0.99, critical=29, power=tail(29, 30, 0.99)),
            dict(
                n=160,
                p0=0.02,
                p1=0.001,
                critical=0,
                power=tail(0, 160, 0.001, upper=False),
            ),
        ],
    )
    write_json(PUBLIC / "results/summary.json", summary)
    csv_table("thresholds.csv", curves)
    flat = []
    for c in conditions:
        for name, m in c["metrics"].items():
            flat.append(
                {k: v for k, v in c.items() if k != "metrics"} | dict(metric=name, **m)
            )
    csv_table("conditions.csv", flat)
    from report import main as render_report

    render_report()
    print({k: v["verdict"] for k, v in hypotheses.items()})


if __name__ == "__main__":
    main()
