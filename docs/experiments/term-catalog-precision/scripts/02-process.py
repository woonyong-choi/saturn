"""Aggregate raw occurrences into pairs, draw the labeling samples, and write the sheet.

Inputs: data/raw/{paren,comment}-{run}.jsonl and $TERM_CATALOG_PRIVATE_DIR/cooc-{run}.jsonl.
Outputs: data/processed/*.csv (public pairs and samples) and private co-occurrence
pairs, samples, and the labeling sheet (written once, never overwritten).
"""

import csv
import hashlib
import json
import os
import random
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np

EXPERIMENT_DIR = Path(__file__).resolve().parent.parent
RAW_DIR = EXPERIMENT_DIR / "data" / "raw"
PROCESSED_DIR = EXPERIMENT_DIR / "data" / "processed"
PRIVATE_DIR = Path(
    os.environ.get(
        "TERM_CATALOG_PRIVATE_DIR",
        "~/workspace/woon/.local/orchestration/saturn-experiments/term-catalog-precision/raw",
    )
).expanduser()
SEED = 127
SAMPLE_SIZE = 200
MIN_EVIDENCE = 3
TOP_PER_WORD = 5
CHUNK_TURNS = 4096


def latest_run_id():
    runs = sorted(p.name[len("paren-"):-len(".jsonl")] for p in RAW_DIR.glob("paren-*.jsonl"))
    if not runs:
        raise SystemExit("no raw paren file; run ./run.sh collect first")
    return runs[-1]


def read_jsonl(path):
    with path.open(encoding="utf-8") as handle:
        return [json.loads(line) for line in handle]


def write_csv(path, header, rows):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="") as out:
        writer = csv.writer(out, lineterminator="\n")
        writer.writerow(header)
        writer.writerows(rows)


def aggregate_paren(rows):
    pairs = defaultdict(lambda: {"evidence": 0, "patterns": Counter(), "examples": []})
    for row in rows:
        pair = pairs[(row["ko"], row["en"])]
        pair["evidence"] += 1
        pair["patterns"][row["pattern"]] += 1
        if len(pair["examples"]) < 2 and row["sentence"] not in pair["examples"]:
            pair["examples"].append(row["sentence"])
    return pairs


def aggregate_comment(rows, variant):
    pairs = defaultdict(lambda: {"evidence": 0, "keywords": Counter(), "examples": []})
    for row in rows:
        if row["variant"] != variant:
            continue
        pair = pairs[(row["ko"], row["en"])]
        pair["evidence"] += 1
        pair["keywords"][row["keyword"]] += 1
        example = f"{row['comment']} / {row['definition']}"
        if len(pair["examples"]) < 2 and example not in pair["examples"]:
            pair["examples"].append(example)
    return pairs


def cooc_pairs(turns):
    """Per project co-occurrence counts and Dice for words seen in three or more turns."""
    by_project = defaultdict(list)
    for turn in turns:
        by_project[turn["project"]].append(turn)
    result = []
    for project in sorted(by_project):
        project_turns = by_project[project]
        ko_count = Counter(w for t in project_turns for w in t["ko"])
        en_count = Counter(w for t in project_turns for w in t["en"])
        ko_vocab = sorted(w for w, n in ko_count.items() if n >= MIN_EVIDENCE)
        en_vocab = sorted(w for w, n in en_count.items() if n >= MIN_EVIDENCE)
        if not ko_vocab or not en_vocab:
            continue
        ko_index = {w: i for i, w in enumerate(ko_vocab)}
        en_index = {w: i for i, w in enumerate(en_vocab)}
        co = np.zeros((len(ko_vocab), len(en_vocab)), dtype=np.float64)
        for start in range(0, len(project_turns), CHUNK_TURNS):
            chunk = project_turns[start:start + CHUNK_TURNS]
            k = np.zeros((len(chunk), len(ko_vocab)), dtype=np.float32)
            e = np.zeros((len(chunk), len(en_vocab)), dtype=np.float32)
            for row, turn in enumerate(chunk):
                for w in turn["ko"]:
                    if w in ko_index:
                        k[row, ko_index[w]] = 1.0
                for w in turn["en"]:
                    if w in en_index:
                        e[row, en_index[w]] = 1.0
            co += (k.T @ e).astype(np.float64)
        for i, j in zip(*np.nonzero(co >= MIN_EVIDENCE)):
            ko, en, together = ko_vocab[i], en_vocab[j], int(round(co[i, j]))
            dice = 2.0 * together / (ko_count[ko] + en_count[en])
            result.append([project, ko, en, together, ko_count[ko], en_count[en], round(dice, 6)])
    ranked = defaultdict(list)
    for row in result:
        ranked[(row[0], row[1])].append(row)
    candidates = []
    for key in sorted(ranked):
        rows = sorted(ranked[key], key=lambda r: (-r[6], -r[3], r[2]))
        candidates.extend(rows[:TOP_PER_WORD])
    return result, candidates


def sample(population, size):
    population = sorted(population)
    if len(population) <= size:
        return population
    return sorted(random.Random(SEED).sample(population, size))


def row_id(ko, en):
    return "r" + hashlib.sha256(f"{ko}|{en}".encode("utf-8")).hexdigest()[:8]


def main():
    run_id = latest_run_id()
    paren = aggregate_paren(read_jsonl(RAW_DIR / f"paren-{run_id}.jsonl"))
    comment_rows = read_jsonl(RAW_DIR / f"comment-{run_id}.jsonl")
    comment = aggregate_comment(comment_rows, "strict")
    extended = aggregate_comment(comment_rows, "extended")

    write_csv(PROCESSED_DIR / "pairs-paren.csv",
              ["ko", "en", "evidence", "ko_paren_en", "en_paren_ko", "code_adjacent", "example_1", "example_2"],
              [[ko, en, p["evidence"], p["patterns"]["ko_paren_en"], p["patterns"]["en_paren_ko"],
                p["patterns"]["code_adjacent"], *(p["examples"] + ["", ""])[:2]]
               for (ko, en), p in sorted(paren.items())])
    for name, pairs in (("pairs-comment.csv", comment), ("pairs-comment-extended.csv", extended)):
        write_csv(PROCESSED_DIR / name, ["ko", "en", "evidence", "keywords", "example_1", "example_2"],
                  [[ko, en, p["evidence"], " ".join(sorted(p["keywords"])), *(p["examples"] + ["", ""])[:2]]
                   for (ko, en), p in sorted(pairs.items())])

    turns = read_jsonl(PRIVATE_DIR / f"cooc-{run_id}.jsonl")
    all_cooc, candidates = cooc_pairs(turns)
    header = ["project", "ko", "en", "together", "ko_turns", "en_turns", "dice"]
    write_csv(PRIVATE_DIR / f"cooc-pairs-{run_id}.csv", header, all_cooc)
    write_csv(PRIVATE_DIR / f"cooc-candidates-{run_id}.csv", header, candidates)

    strata = {
        "paren_random": sample(paren.keys(), SAMPLE_SIZE),
        "paren_ev3": sorted(k for k, p in paren.items() if p["evidence"] >= MIN_EVIDENCE),
        "comment_random": sample(comment.keys(), SAMPLE_SIZE),
        "comment_extended": sample(set(extended) - set(comment), SAMPLE_SIZE),
    }
    write_csv(PROCESSED_DIR / "samples.csv", ["stratum", "ko", "en"],
              [[s, ko, en] for s, keys in strata.items() for ko, en in keys])
    cooc_sample = sample([tuple(r[:3]) for r in candidates], SAMPLE_SIZE)
    write_csv(PRIVATE_DIR / f"samples-cooc-{run_id}.csv", ["stratum", "project", "ko", "en"],
              [["cooc_random", *key] for key in cooc_sample])

    sheet_path = PRIVATE_DIR / f"label-sheet-{run_id}.csv"
    if not sheet_path.exists():
        words = {pair for keys in strata.values() for pair in keys} | {(ko, en) for _, ko, en in cooc_sample}
        rows = sorted([row_id(ko, en), ko, en, ""] for ko, en in words)
        random.Random(SEED).shuffle(rows)
        write_csv(sheet_path, ["row_id", "ko", "en", "label"], rows)
    print(f"{run_id}: paren {len(paren)}, comment {len(comment)}, extended {len(extended)}, "
          f"cooc candidates {len(candidates)}")


if __name__ == "__main__":
    main()
