"""Collect term pair candidates for the term catalog precision experiment.

Commands:
    mine            Mine paren and comment occurrences from the Saturn repository
                    and co-occurrence turns from local Claude Code records.
    labels SHEET    Split a filled labeling sheet into raw label files.

Public raw files go to data/raw/. Private raw files (co-occurrence turns,
labeling sheet, co-occurrence labels) go to $TERM_CATALOG_PRIVATE_DIR only.
"""

import csv
import datetime as dt
import hashlib
import json
import os
import platform
import re
import subprocess
import sys
from pathlib import Path

EXPERIMENT_DIR = Path(__file__).resolve().parent.parent
REPO_ROOT = EXPERIMENT_DIR.parent.parent.parent
RAW_DIR = EXPERIMENT_DIR / "data" / "raw"
PRIVATE_DIR = Path(
    os.environ.get(
        "TERM_CATALOG_PRIVATE_DIR",
        "~/workspace/woon/.local/orchestration/saturn-experiments/term-catalog-precision/raw",
    )
).expanduser()
CLAUDE_PROJECTS = Path(os.environ.get("CLAUDE_PROJECTS_DIR", "~/.claude/projects")).expanduser()
COOC_CUTOFF = "2026-10-01T00:00:00Z"
SEED = 127

HANGUL_RUN = re.compile(r"[가-힣]+")
IDENT = r"[A-Za-z_][A-Za-z0-9_.:/<>-]*"
IDENT_FULL = re.compile(rf"^{IDENT}$")
KO_PAREN_EN = re.compile(rf"([가-힣]+)\(\s*`?({IDENT})`?\s*\)")
EN_PAREN_KO = re.compile(rf"(?<![A-Za-z0-9_`])`?({IDENT})`?\(([가-힣]+(?: [가-힣]+){{0,2}})\)")
CODE_SPAN = re.compile(r"`([^`]+)`")
DEF_STRICT = re.compile(
    r"^\s*(?:(?:pub(?:\([^)]*\))?|async|const|unsafe|export|default|static|extern(?:\s+\"[^\"]*\")?)\s+)*"
    r"(fn|def|function|class)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
DEF_EXT = re.compile(
    r"^\s*(?:(?:pub(?:\([^)]*\))?|async|unsafe|export|default|static|extern(?:\s+\"[^\"]*\")?)\s+)*"
    r"(?:const\s+(?=fn\b))?"
    r"(fn|def|function|class|struct|enum|trait|type|const|mod)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
COMMENT_PREFIX = re.compile(r"^\s*(//+!?|/\*+|\*|#(?![\[!]))\s*(.*)$")
ATTRIBUTE_LINE = re.compile(r"^\s*#\[")
CODE_SUFFIXES = (".rs", ".py", ".js", ".ts", ".sh", ".toml")
PATH_TOKEN = re.compile(r"[A-Za-z0-9_./~-]*(?:/[A-Za-z0-9_.~-]+|\.[A-Za-z][A-Za-z0-9]{0,4})$")
WORD_PIECE = re.compile(r"[A-Z]+(?=[A-Z][a-z])|[A-Z]?[a-z]+|[A-Z]+")
TOOL_PATH_KEYS = ("file_path", "path", "notebook_path", "pattern", "glob")
USER_TEXT_SKIP = ("<command-", "<local-command-", "<system-reminder>", "Caveat:")

# Longest first. A suffix is removed only when at least two syllables remain.
PARTICLES = (
    "에서는", "으로는", "에서도", "이라는", "에게서",
    "에서", "으로", "에게", "까지", "부터", "처럼", "보다", "마다", "이나", "이다",
    "이고", "이면", "이며", "에는", "에도", "과는", "와는", "라는", "하고", "만큼", "대로",
    "은", "는", "이", "가", "을", "를", "의", "에", "로", "와", "과", "도", "만",
)
FUNCTION_WORDS = frozenset(PARTICLES) | {"하는", "한다", "하면", "하여", "해서", "된다", "있다", "없다"}


def strip_particle(word):
    """Return the Korean word without a trailing particle, or None if too short."""
    if word in FUNCTION_WORDS:
        return None
    for particle in PARTICLES:
        if word.endswith(particle) and len(word) - len(particle) >= 2:
            word = word[: -len(particle)]
            break
    if not 2 <= len(word) <= 10:
        return None
    return word


def korean_phrase(text):
    """Normalize a Korean phrase of up to three words; strip the last word's particle."""
    words = text.split()
    last = strip_particle(words[-1]) if len(words[-1]) >= 2 else None
    if last is None:
        return None
    phrase = " ".join(words[:-1] + [last])
    if not 2 <= len(phrase.replace(" ", "")) <= 10:
        return None
    return phrase


def english_ident(text):
    ident = text.strip().lower()
    if not 2 <= len(ident) <= 40 or not IDENT_FULL.match(ident):
        return None
    return ident


def word_pieces(text):
    """Split text into lowercase latin word pieces of three or more letters."""
    pieces = set()
    for chunk in re.split(r"[^A-Za-z0-9]+", text):
        for piece in WORD_PIECE.findall(chunk):
            if len(piece) >= 3:
                pieces.add(piece.lower())
    return pieces


def git(*args):
    return subprocess.run(
        ["git", "-C", str(REPO_ROOT), *args], check=True, capture_output=True, text=True
    ).stdout


def tracked_files(commit):
    return git("ls-tree", "-r", "--name-only", commit).splitlines()


def file_text(commit, path):
    return git("show", f"{commit}:{path}")


def utc_now():
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def write_jsonl(path, rows):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as out:
        for row in rows:
            out.write(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n")


def is_paren_doc(path):
    if path.startswith(("docs/archive/", "docs/experiments/")):
        return False
    return path in ("README.ko.md", "AGENTS.md") or (path.startswith("docs/") and path.endswith(".md"))


def mine_paren(commit, run_id, ts):
    rows = []
    for path in sorted(p for p in tracked_files(commit) if is_paren_doc(p)):
        in_fence = False
        for number, line in enumerate(file_text(commit, path).splitlines(), start=1):
            if line.lstrip().startswith("```"):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            for pattern, ko, en in paren_matches(line):
                rows.append({
                    "run_id": run_id, "trial_id": f"paren-{len(rows) + 1:05d}", "condition": "paren",
                    "ts_utc": ts, "input_id": f"{path}:{number}", "pattern": pattern,
                    "ko": ko, "en": en, "sentence": line.strip()[:300],
                })
    return rows


def paren_matches(line):
    found = []
    for match in KO_PAREN_EN.finditer(line):
        ko, en = strip_particle(match.group(1)), english_ident(match.group(2))
        if ko and en:
            found.append(("ko_paren_en", ko, en))
    for match in EN_PAREN_KO.finditer(line):
        en, ko = english_ident(match.group(1)), korean_phrase(match.group(2))
        if ko and en:
            found.append(("en_paren_ko", ko, en))
    for match in CODE_SPAN.finditer(line):
        en = english_ident(match.group(1))
        if en is None:
            continue
        before = re.search(r"([가-힣]+) ?$", line[: match.start()])
        after = re.match(r" ?([가-힣]+)", line[match.end():])
        for side in (before, after):
            ko = strip_particle(side.group(1)) if side else None
            if ko:
                found.append(("code_adjacent", ko, en))
    return found


def mine_comment(commit, run_id, ts):
    rows = []
    for path in sorted(p for p in tracked_files(commit) if p.endswith(CODE_SUFFIXES)):
        lines = file_text(commit, path).splitlines()
        for index, line in enumerate(lines):
            for row in comment_pairs(lines, index):
                row.update({
                    "run_id": run_id, "trial_id": f"comment-{len(rows) + 1:05d}", "condition": "comment",
                    "ts_utc": ts, "input_id": f"{path}:{index + 1}",
                })
                rows.append(row)
    return rows


def comment_pairs(lines, index):
    """Pairs for a definition line at index: strict rule and extended variant."""
    pairs = []
    definition = lines[index]
    previous = lines[index - 1] if index > 0 else ""
    strict = DEF_STRICT.match(definition)
    comment = COMMENT_PREFIX.match(previous)
    if strict and comment:
        first = comment.group(2).split()[:1]
        lead = HANGUL_RUN.match(first[0]) if first else None
        ko = strip_particle(lead.group(0)) if lead else None
        en = english_ident(strict.group(2))
        if ko and en:
            pairs.append({"variant": "strict", "keyword": strict.group(1), "ko": ko, "en": en,
                          "comment": previous.strip()[:300], "definition": definition.strip()[:300]})
    extended = DEF_EXT.match(definition)
    if not extended:
        return pairs
    block = []
    cursor = index - 1
    while cursor >= 0 and ATTRIBUTE_LINE.match(lines[cursor]):
        cursor -= 1
    while cursor >= 0 and COMMENT_PREFIX.match(lines[cursor]) and not ATTRIBUTE_LINE.match(lines[cursor]):
        block.append(COMMENT_PREFIX.match(lines[cursor]).group(2))
        cursor -= 1
    en = english_ident(extended.group(2))
    seen = set()
    for text in reversed(block):
        for run in HANGUL_RUN.findall(text):
            ko = strip_particle(run)
            if ko and en and ko not in seen:
                seen.add(ko)
                pairs.append({"variant": "extended", "keyword": extended.group(1), "ko": ko, "en": en,
                              "comment": " ".join(reversed(block)).strip()[:300],
                              "definition": definition.strip()[:300]})
    return pairs


def user_text(record):
    if record.get("isMeta"):
        return None
    content = record.get("message", {}).get("content")
    if isinstance(content, str):
        text = content
    elif isinstance(content, list):
        if any(isinstance(b, dict) and b.get("type") == "tool_result" for b in content):
            return None
        text = "\n".join(b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text")
    else:
        return None
    text = text.strip()
    if not text or text.startswith(USER_TEXT_SKIP):
        return None
    return text


def tool_pieces(tool_input):
    pieces = set()
    if not isinstance(tool_input, dict):
        return pieces
    for key in TOOL_PATH_KEYS:
        value = tool_input.get(key)
        if isinstance(value, str):
            pieces |= word_pieces(value)
    command = tool_input.get("command")
    if isinstance(command, str):
        for token in re.split(r"[\s'\"=;|&<>()]+", command):
            if token and PATH_TOKEN.fullmatch(token) and re.search(r"[A-Za-z]", token):
                pieces |= word_pieces(token)
    return pieces


def short_hash(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()[:12]


def mine_cooc(run_id, ts, stats):
    rows = []
    files = sorted(p for p in CLAUDE_PROJECTS.rglob("*.jsonl") if "subagents" not in p.parts)
    for path in files:
        project = short_hash(path.relative_to(CLAUDE_PROJECTS).parts[0])
        session = short_hash(path.stem)
        turn = None
        turn_index = 0
        try:
            handle = path.open(encoding="utf-8")
        except OSError:
            stats["unreadable_files"] += 1
            continue
        with handle:
            for line in handle:
                try:
                    record = json.loads(line)
                except json.JSONDecodeError:
                    stats["bad_lines"] += 1
                    continue
                if not isinstance(record, dict) or record.get("isSidechain"):
                    continue
                kind = record.get("type")
                if kind == "user":
                    text = user_text(record)
                    if text is None:
                        continue
                    if turn is not None:
                        rows.append(turn)
                    turn = None
                    stamp = record.get("timestamp", "")
                    if not stamp or stamp >= COOC_CUTOFF:
                        continue
                    turn_index += 1
                    ko = sorted({w for w in (strip_particle(r) for r in HANGUL_RUN.findall(text)) if w})
                    turn = {"run_id": run_id, "trial_id": f"{session}-{turn_index:05d}", "condition": "cooc",
                            "ts_utc": ts, "input_id": f"{project}/{session}/{turn_index}", "project": project,
                            "turn_ts": stamp[:10], "ko": ko, "en": []}
                elif kind == "assistant" and turn is not None:
                    content = record.get("message", {}).get("content")
                    if isinstance(content, list):
                        found = set(turn["en"])
                        for block in content:
                            if isinstance(block, dict) and block.get("type") == "tool_use":
                                found |= tool_pieces(block.get("input"))
                        turn["en"] = sorted(found)
        if turn is not None:
            rows.append(turn)
    stats["files"] = len(files)
    for index, row in enumerate(rows, start=1):
        row["trial_id"] = f"cooc-{index:06d}"
    return rows


def project_description(commit, run_id, ts):
    """First plain introduction paragraph of README.md, used as the judge state description."""
    for number, line in enumerate(file_text(commit, "README.md").splitlines(), start=1):
        text = line.strip()
        if len(text) >= 80 and not text.startswith(("#", "<", ">", "|", "-", "!", "[", "`")):
            return [{"run_id": run_id, "trial_id": "readme-1", "condition": "project", "ts_utc": ts,
                     "input_id": f"README.md:{number}", "text": text}]
    raise SystemExit("README.md has no introduction paragraph")


def write_env(commit, run_id, ts):
    import numpy
    memory = subprocess.run(["sysctl", "-n", "hw.memsize"], capture_output=True, text=True).stdout.strip()
    cpu = subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"], capture_output=True, text=True).stdout.strip()
    env = {
        "os": f"{platform.system()} {platform.release()} ({platform.mac_ver()[0]})",
        "cpu": cpu or platform.processor(),
        "memory_bytes": int(memory) if memory.isdigit() else None,
        "tools": {
            "python": platform.python_version(),
            "numpy": numpy.__version__,
            "git": git("--version").strip(),
        },
        "model": "해당 없음. judge를 호출하지 않았다.",
        "run_date": ts[:10],
        "run_id": run_id,
        "commit": commit,
        "seed": SEED,
        "cooc_cutoff_utc": COOC_CUTOFF,
    }
    (EXPERIMENT_DIR / "env.json").write_text(json.dumps(env, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def mine():
    commit = os.environ.get("SOURCE_COMMIT") or git("rev-parse", "origin/main").strip()
    ts = utc_now()
    run_id = f"{ts.replace('-', '').replace(':', '')}-{commit[:7]}"
    stats = {"unreadable_files": 0, "bad_lines": 0, "files": 0}
    write_jsonl(RAW_DIR / f"paren-{run_id}.jsonl", mine_paren(commit, run_id, ts))
    write_jsonl(RAW_DIR / f"comment-{run_id}.jsonl", mine_comment(commit, run_id, ts))
    write_jsonl(RAW_DIR / f"readme-{run_id}.jsonl", project_description(commit, run_id, ts))
    PRIVATE_DIR.mkdir(parents=True, exist_ok=True)
    write_jsonl(PRIVATE_DIR / f"cooc-{run_id}.jsonl", mine_cooc(run_id, ts, stats))
    (PRIVATE_DIR / f"collect-log-{run_id}.json").write_text(json.dumps(stats, indent=2) + "\n", encoding="utf-8")
    write_env(commit, run_id, ts)
    print(run_id)


def import_labels(sheet_path):
    """Split the filled sheet: public pairs to data/raw, the rest to the private folder."""
    sheet = list(csv.DictReader(Path(sheet_path).open(encoding="utf-8")))
    run_id = re.search(r"sheet-(.+)\.csv$", str(sheet_path)).group(1)
    public_pairs = set()
    for name in (f"paren-{run_id}.jsonl", f"comment-{run_id}.jsonl"):
        for line in (RAW_DIR / name).open(encoding="utf-8"):
            row = json.loads(line)
            public_pairs.add((row["ko"], row["en"]))
    ts = utc_now()
    public, private = [], []
    for row in sheet:
        label = row["label"].strip()
        if label not in ("same", "different", "unclear", ""):
            sys.exit(f"bad label {label!r} in row {row['row_id']}")
        record = {"run_id": run_id, "trial_id": row["row_id"], "condition": "label", "ts_utc": ts,
                  "input_id": row["row_id"], "ko": row["ko"], "en": row["en"], "label": label or None}
        (public if (row["ko"], row["en"]) in public_pairs else private).append(record)
    write_jsonl(RAW_DIR / f"labels-public-{run_id}.jsonl", public)
    write_jsonl(PRIVATE_DIR / f"labels-cooc-{run_id}.jsonl", private)
    print(f"public {len(public)}, private {len(private)}")


def main():
    if len(sys.argv) >= 2 and sys.argv[1] == "mine":
        mine()
    elif len(sys.argv) == 3 and sys.argv[1] == "labels":
        import_labels(sys.argv[2])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
