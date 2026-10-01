"""공개 합성 작업 생성. 같은 시드와 번호는 늘 같은 작업 폴더, 지시문, 정답 사실을 만든다.

작업 하나는 파일 읽기 4번, 명령 실행 3번(스크립트 2번과 테스트 1번), 설정 파일 수정 1번이다.
정답 사실은 읽기와 명령 출력에 심은 값 13개이고, 수정은 정답 사실 없이 도구 종류와 경로, 줄 수만 잰다.
"""
import random

WORDS = [
    "kestrel", "marlin", "juniper", "obsidian", "lantern", "cobalt", "saffron", "tundra",
    "heron", "quartz", "willow", "ember", "basalt", "cypress", "falcon", "garnet",
    "harbor", "indigo", "jasper", "kelp", "lichen", "meadow", "nectar", "onyx",
]
LABELS = [
    "vendor code", "release tag", "backup site", "queue name", "audit ticket", "cache key",
    "owner alias", "region label", "batch mark", "shelf code", "route name", "cost center",
]
NOTE_FILES = ["a", "b", "c", "d"]
FILLER = [
    "Remember to keep this folder tidy.",
    "Nothing in this line matters.",
    "Meeting notes are stored elsewhere.",
    "The weekly sync moved to Thursday.",
    "Draft wording is still under review.",
    "Archive older entries monthly.",
]
UNIT_PAGE = 7


def _token(rng, used):
    while True:
        token = f"{rng.choice(WORDS)}_{rng.randint(1000, 9999)}"
        if token not in used:
            used.add(token)
            return token


def make_task(seed, index):
    rng = random.Random(f"{seed}-{index}")
    used = set()
    labels = rng.sample(LABELS, 8)
    files = {}
    facts = []
    items = []
    for number, name in enumerate(NOTE_FILES):
        pair = labels[number * 2:number * 2 + 2]
        path = f"notes/{name}.txt"
        lines = [rng.choice(FILLER) for _ in range(UNIT_PAGE)]
        positions = sorted(rng.sample(range(UNIT_PAGE), 2))
        item_id = f"r{number + 1}"
        for position, label in zip(positions, pair):
            token = _token(rng, used)
            lines[position] = f"{label}: {token}"
            facts.append({
                "id": f"{item_id}-{len(facts)}", "item": item_id, "marker": token,
                "question": f"What is the {label} in {path}?",
            })
        files[path] = "\n".join(lines) + "\n"
        items.append({"id": item_id, "kind": "FileRead", "path": path, "command": None, "exit_code": None})

    region, batch, ticket = (_token(rng, used) for _ in range(3))
    files["scripts/summary.py"] = (
        "print('summary ready')\n"
        f"print('region label: {region}')\n"
        f"print('batch mark: {batch}')\n"
    )
    facts.append({"id": "c1-a", "item": "c1", "marker": region, "question": "What region label did scripts/summary.py print?"})
    facts.append({"id": "c1-b", "item": "c1", "marker": batch, "question": "What batch mark did scripts/summary.py print?"})
    items.append({"id": "c1", "kind": "Shell", "path": None, "command": "python3 scripts/summary.py", "exit_code": 0})

    files["scripts/check.py"] = (
        "import sys\n"
        f"print('check failed: rule {ticket} is not satisfied')\n"
        "sys.exit(3)\n"
    )
    facts.append({"id": "c2-a", "item": "c2", "marker": ticket, "question": "Which rule did scripts/check.py report as not satisfied?"})
    items.append({"id": "c2", "kind": "Shell", "path": None, "command": "python3 scripts/check.py", "exit_code": 3})

    mode, retries = rng.sample(WORDS, 2)
    files["config/app.cfg"] = "name=demo\nmode=draft\nretries=1\nlevel=2\n"
    items.append({
        "id": "e1", "kind": "FileEdit", "path": "config/app.cfg", "command": None, "exit_code": None,
        "added": 2, "removed": 2, "new_text": f"mode={mode}\nretries={retries}\n",
    })

    failing = [_token(rng, used) for _ in range(2)]
    cases = "".join(
        f"    def test_rule_{token}(self):\n        self.assertEqual(1, 2)\n\n" for token in failing
    )
    files["tests/__init__.py"] = ""
    files["tests/test_rules.py"] = (
        "import unittest\n\n\nclass Rules(unittest.TestCase):\n"
        "    def test_ok_one(self):\n        self.assertTrue(True)\n\n"
        "    def test_ok_two(self):\n        self.assertTrue(True)\n\n" + cases
    ).rstrip("\n") + "\n"
    for number, token in enumerate(failing):
        facts.append({
            "id": f"t1-{number}", "item": "t1", "marker": f"test_rule_{token}",
            "question": f"Which failing test in tests/test_rules.py has a name containing {token.split('_')[0]}? Answer with the full test name.",
        })
    items.append({"id": "t1", "kind": "TestRun", "path": None, "command": "python3 -m unittest tests.test_rules", "exit_code": 1})

    steps = [
        *[f"Read {item['path']}." for item in items if item["kind"] == "FileRead"],
        "Run `python3 scripts/summary.py`.",
        "Run `python3 scripts/check.py`.",
        "Edit config/app.cfg so that the line `mode=draft` becomes `mode=" + mode
        + "` and the line `retries=1` becomes `retries=" + retries + "`, in one edit.",
        "Run `python3 -m unittest tests.test_rules`.",
    ]
    instruction = (
        "Work in the current folder. Do the following steps in order, one tool call per step, "
        "and do nothing else. Read each file in its own call. Change files only with your file "
        "editing tool, not with a shell command. A step that fails is not a problem, continue "
        "with the next step. When all steps are done, reply with only the word done.\n\n"
        + "\n".join(f"{number}. {text}" for number, text in enumerate(steps, 1))
        + "\n"
    )
    return {
        "task_id": f"t{index:02d}", "seed": seed, "instruction": instruction,
        "files": files, "items": items, "facts": facts,
    }
