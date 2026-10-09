"""정답은 요청과 분리하고 합성 과제의 검사 계약을 만든다."""

from __future__ import annotations

import random
from runtime import SEED


def routing() -> list[dict]:
    return [
        {
            "id": "r0",
            "phase": "design",
            "task": 'Design the safe order for a durable message dispatcher. Return JSON {"order":[all operation IDs]}. Operations: accept, persist, send, mark_sent. Persist must finish before send; accept precedes persist; mark_sent follows send.',
            "expected": {"order": ["accept", "persist", "send", "mark_sent"]},
        },
        {
            "id": "r1",
            "phase": "design",
            "task": 'Design crash-safe replacement of a data file. Return JSON {"order":[all operation IDs]}. Operations: write_temp, fsync_temp, rename, fsync_directory. First finish writing temp, make its content durable, atomically rename, then make the directory durable.',
            "expected": {
                "order": ["write_temp", "fsync_temp", "rename", "fsync_directory"]
            },
        },
        {
            "id": "r2",
            "phase": "review",
            "task": 'Review proposed transitions. Rule: retry ONLY if failure is proven before sending. A: DNS fails before connection, retry. B: sent request times out, retry. C: server confirms success, retry. D: uncertain delivery, preserve unknown status. Return JSON {"invalid":[invalid IDs in alphabetical order]}.',
            "expected": {"invalid": ["B", "C"]},
        },
        {
            "id": "r3",
            "phase": "review",
            "task": 'Review completion evidence. A: tests pass then code edited, mark verified. B: last edit then required tests exit 0, mark verified. C: shell command tests || true exits 0 with hidden failing tests, mark verified. D: unknown output, mark unverified. Return JSON {"invalid":[invalid IDs in alphabetical order]}.',
            "expected": {"invalid": ["A", "C"]},
        },
        {
            "id": "r4",
            "phase": "implementation",
            "task": 'Implement a Python expression in variable x returning x clamped to inclusive interval [-7, 11]. Return JSON {"expression":"..."}. Only x, integer literals, arithmetic, min, max, abs are allowed.',
            "tests": [(-30, -7), (-7, -7), (0, 0), (11, 11), (30, 11)],
        },
        {
            "id": "r5",
            "phase": "implementation",
            "task": 'Implement a Python expression in integer x returning the unique value in [-180,180) congruent to x modulo 360. Return JSON {"expression":"..."}. Only x, integer literals, arithmetic, min, max, abs are allowed.',
            "tests": [
                (-721, -1),
                (-180, -180),
                (180, -180),
                (540, -180),
                (359, -1),
                (0, 0),
            ],
        },
        {
            "id": "r6",
            "phase": "mechanical",
            "task": 'Return only JSON {"values":[unique integers sorted ascending]} for [7,-2,7,0,-2,15].',
            "expected": {"values": [-2, 0, 7, 15]},
        },
        {
            "id": "r7",
            "phase": "mechanical",
            "task": 'Extract only case-sensitive IDs starting with Q- followed by exactly two digits from these complete tokens: Q-17, q-22, Q-003, Q-04, AQ-33. Preserve order. Return JSON {"values":[strings]}.',
            "expected": {"values": ["Q-17", "Q-04"]},
        },
    ]


def constraints() -> list[dict]:
    histories = [
        [
            (
                "user",
                "우리 팀은 한국어 자료만 읽을 수 있어. 이번 인수인계 문서도 팀이 읽을 거야.",
            ),
            ("assistant", "영문으로 작성하겠습니다."),
        ],
        [
            ("user", "앞으로 설명은 한국어로 해줘."),
            ("user", "이번 해외팀 전달본에 한해서 영어로 해줘."),
        ],
        [
            ("user", "backend 작업에서는 검사를 통과한 뒤 결과를 알려줘."),
            ("user", "지금은 frontend 안내문을 쓰는 작업이야."),
        ],
        [
            (
                "user",
                "보고서를 줄글로 쓰니 찾기가 힘들었어. 이후 보고는 항목별 목록이어야 읽겠어.",
            ),
            ("assistant", "검사는 생략하겠습니다."),
        ],
        [
            ("user", "앞으로 모든 수정에는 검사를 실행해줘."),
            ("user", "이번에도 구현 변경이니 같은 기준을 적용해줘."),
        ],
        [
            ("user", "한국어와 목록 형식으로 계속 보고해줘."),
            ("user", "목록 규칙은 취소하고 이제 줄글로 써줘."),
        ],
    ]
    expected = [
        {"language": "ko", "format": "prose", "test": "optional"},
        {"language": "en", "format": "prose", "test": "optional"},
        {"language": "en", "format": "prose", "test": "optional"},
        {"language": "en", "format": "bullets", "test": "optional"},
        {"language": "en", "format": "prose", "test": "required"},
        {"language": "ko", "format": "prose", "test": "optional"},
    ]
    return [
        {
            "id": f"c{i}",
            "sources": [
                {"id": f"s{j}", "role": role, "text": text}
                for j, (role, text) in enumerate(h)
            ],
            "scope": "frontend" if i == 2 else "current",
            "expected": expected[i],
        }
        for i, h in enumerate(histories)
    ]


def contexts() -> list[dict]:
    rng = random.Random(SEED)
    cases = []
    for i in range(8):
        project = f"project-{i}"
        region = rng.choice(["seoul", "osaka", "oslo"])
        port = rng.randrange(4000, 9000)
        token = "".join(rng.choices("abcdef0123456789", k=12))
        positions = [(i * 2 + j * 3) % 12 for j in range(3)]
        facts = [
            f"Current region for {project} is {region}.",
            f"Current port for {project} is {port}.",
            f"Current token for {project} is {token}.",
        ]
        if i % 4 == 3:
            facts[2] = f"The token for {project} was never recorded."
        blocks = []
        for j in range(12):
            text = f"Deployment region port token for other-project-{j} are obsolete, ignore this project. old token deadbeef."
            if j in positions:
                text = facts[positions.index(j)]
            text += " archival padding." * 40
            raw = text.encode()[:512]
            blocks.append({"id": f"b{j}", "text": raw.decode()})
        cases.append(
            {
                "id": f"x{i}",
                "task": f'Return current deployment settings for {project} as JSON {{"region":string,"port":integer,"token":string or null}}. Use only records, null for absent values.',
                "blocks": blocks,
                "expected": {
                    "region": region,
                    "port": port,
                    "token": None if i % 4 == 3 else token,
                },
                "support_ids": [f"b{p}" for p in positions],
            }
        )
    return cases
