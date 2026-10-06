"""과제 계열. 계열마다 시작 저장소, 기준 해법, 시험 입력, 한국어·혼합어 지시문을 매개변수로 만든다.

채점은 에이전트가 고친 저장소와 기준 해법 저장소에 같은 호출을 하고 결과(또는 예외 종류)를 비교한다.
시험 입력과 기준 해법은 에이전트의 작업 폴더에 들어가지 않는다.
"""

from __future__ import annotations

import random
from decimal import ROUND_HALF_UP, Decimal  # noqa: F401  (기준 해법 문자열이 쓴다)

# ---------------------------------------------------------------- 개발 계열

RETRY_INITIAL = '''import time


def call_with_retry(func, retries, base_delay, max_delay, sleep=time.sleep, retry_on=(Exception,)):
    """func를 호출하고 실패하면 다시 시도한다."""
    last = None
    for attempt in range(retries):
        try:
            return func()
        except retry_on as error:
            last = error
            sleep(base_delay)
    return None
'''

RETRY_SOLUTION = '''import time


def call_with_retry(func, retries, base_delay, max_delay, sleep=time.sleep, retry_on=(Exception,)):
    """func를 호출하고 실패하면 다시 시도한다."""
    if retries < 0:
        raise ValueError("retries must not be negative")
    for attempt in range(retries + 1):
        try:
            return func()
        except retry_on:
            if attempt == retries:
                raise
            sleep(min(base_delay * FACTOR ** attempt, max_delay))
'''


def retry_family(p: dict) -> dict:
    solution = RETRY_SOLUTION.replace("FACTOR", str(p["factor"]))
    return dict(
        initial={"net/__init__.py": "", "net/retry.py": RETRY_INITIAL, "README.md": "# net\n\n네트워크 호출 도우미.\n"},
        solution={"net/retry.py": solution},
        target="net/retry.py",
        cases=lambda rng: retry_cases(p, rng),
        spec_ko=(
            "net/retry.py의 call_with_retry를 고쳐 줘. 규칙은 이렇다. retries는 첫 호출 뒤 추가로 시도하는 횟수라서 "
            "전체 시도는 retries+1번이다. 성공하면 func의 반환값을 그대로 돌려준다. 실패하면 다음 시도 전에 "
            f"min(base_delay * {p['factor']}**(k-1), max_delay)초를 sleep으로 기다린다(k는 1부터 세는 재시도 번호). "
            "마지막 시도가 실패하면 기다리지 않고 마지막 예외를 그대로 다시 던진다. retry_on에 없는 예외는 재시도 없이 바로 던진다. "
            "retries가 음수면 ValueError."
        ),
        spec_mixed=(
            "net/retry.py 의 call_with_retry 를 fix 해 줘. retries 는 첫 call 이후의 extra attempt 수라서 total attempts 는 retries+1 이다. "
            "success 면 func 의 return value 를 그대로 return. fail 하면 next attempt 전에 "
            f"min(base_delay * {p['factor']}**(k-1), max_delay) 초를 sleep 인자로 wait 한다(k 는 1부터 세는 retry 번호). "
            "마지막 attempt 가 fail 하면 sleep 없이 last exception 을 re-raise. retry_on 에 없는 exception 은 retry 없이 즉시 raise. "
            "retries 가 negative 면 ValueError."
        ),
    )


class _Flaky:
    def __init__(self, fail_times, exc):
        self.fail_times, self.exc, self.calls = fail_times, exc, 0

    def __call__(self):
        self.calls += 1
        if self.calls <= self.fail_times:
            raise self.exc(f"fail {self.calls}")
        return f"ok-{self.calls}"


def retry_cases(p: dict, rng: random.Random):
    cases = []
    specs = [(0, 0, ValueError), (1, 0, ValueError), (2, 2, ValueError), (3, 5, ValueError), (4, 2, ValueError), (5, 9, KeyError)]
    for _ in range(4):
        specs.append((rng.randint(0, 5), rng.randint(0, 6), ValueError))

    def make(retries, fails, exc, retry_on):
        def run(load, root):
            mod = load("net/retry.py")
            sleeps, flaky = [], _Flaky(fails, exc)
            try:
                value = mod.call_with_retry(flaky, retries, 0.5, p["cap"], sleep=sleeps.append, retry_on=retry_on)
                outcome = ("ok", value)
            except BaseException as error:  # noqa: BLE001
                outcome = ("raise", type(error).__name__, str(error))
            return outcome, flaky.calls, sleeps
        return run

    for i, (retries, fails, exc) in enumerate(specs):
        cases.append((f"retry-{i}", make(retries, fails, exc, (ValueError,) if exc is ValueError else (Exception,))))
    cases.append(("retry-unlisted", make(3, 2, KeyError, (ValueError,))))

    def negative(load, root):
        mod = load("net/retry.py")
        try:
            mod.call_with_retry(lambda: 1, -1, 1, 2, sleep=lambda s: None)
        except BaseException as error:  # noqa: BLE001
            return type(error).__name__
        return "no error"

    cases.append(("retry-negative", negative))
    return cases


PAGE_INITIAL = '''def paginate(items, page=1, per_page=10):
    """목록을 쪽으로 나눈다."""
    start = page * per_page
    chunk = items[start:start + per_page]
    return {
        "items": chunk,
        "page": page,
        "per_page": per_page,
        "total_items": len(items),
        "total_pages": len(items) // per_page,
        "has_next": start + per_page < len(items),
        "has_prev": page > 0,
    }
'''

PAGE_SOLUTION = '''def paginate(items, page=1, per_page=10):
    """목록을 쪽으로 나눈다."""
    if per_page < 1 or page < 1:
        raise ValueError("page and per_page must be at least 1")
    per_page = min(per_page, MAXPP)
    total = len(items)
    pages = -(-total // per_page)
    start = (page - 1) * per_page
    return {
        "items": list(items[start:start + per_page]),
        "page": page,
        "per_page": per_page,
        "total_items": total,
        "total_pages": pages,
        "has_next": page < pages,
        "has_prev": page > 1,
    }
'''


def paginate_family(p: dict) -> dict:
    return dict(
        initial={"listing/__init__.py": "", "listing/pages.py": PAGE_INITIAL, "README.md": "# listing\n"},
        solution={"listing/pages.py": PAGE_SOLUTION.replace("MAXPP", str(p["max_per_page"]))},
        target="listing/pages.py",
        cases=lambda rng: paginate_cases(p, rng),
        spec_ko=(
            "listing/pages.py의 paginate를 고쳐 줘. 쪽 번호는 1부터다. 결과 dict의 키는 그대로 두되 값은 이렇게 맞춘다. "
            "items는 그 쪽의 항목 리스트, total_pages는 올림 나눗셈(항목이 없으면 0), has_next는 page < total_pages, "
            f"has_prev는 page > 1이다. per_page가 {p['max_per_page']}보다 크면 {p['max_per_page']}로 줄이고 결과의 per_page에도 줄인 값을 쓴다. "
            "page나 per_page가 1보다 작으면 ValueError. 마지막 쪽을 지난 page는 items가 빈 리스트일 뿐 오류는 아니다."
        ),
        spec_mixed=(
            "listing/pages.py 의 paginate 를 fix 해 줘. page 번호는 1-based. 결과 dict 의 key 는 그대로 두고 value 만 맞춘다. "
            "items 는 해당 page 의 list, total_pages 는 ceil division(항목 없으면 0), has_next 는 page < total_pages, "
            f"has_prev 는 page > 1. per_page 가 {p['max_per_page']} 보다 크면 {p['max_per_page']} 로 clamp 하고 result 의 per_page 에도 clamp 된 값을 쓴다. "
            "page 나 per_page 가 1 미만이면 ValueError. 마지막 page 를 넘는 page 는 items 가 empty list 일 뿐 error 가 아니다."
        ),
    )


def paginate_cases(p: dict, rng: random.Random):
    cases = []
    argsets = [([], 1, 10), ([1], 1, 10), (list(range(10)), 1, 10), (list(range(11)), 2, 10), (list(range(11)), 3, 10),
               (list(range(25)), 3, 10), (list(range(25)), 1, 0), (list(range(25)), 0, 5), (list(range(500)), 2, 1000),
               (list(range(500)), 2, p["max_per_page"] + 1), (list(range(7)), -1, 3)]
    for _ in range(5):
        n = rng.randint(0, 60)
        argsets.append((list(range(n)), rng.randint(1, 8), rng.randint(1, 15)))

    def make(items, page, per_page):
        def run(load, root):
            mod = load("listing/pages.py")
            try:
                return mod.paginate(items, page, per_page)
            except BaseException as error:  # noqa: BLE001
                return ("raise", type(error).__name__)
        return run

    for i, args in enumerate(argsets):
        cases.append((f"page-{i}", make(*args)))
    return cases


TTL_INITIAL = '''import time


class TTLCache:
    def __init__(self, capacity, ttl, clock=time.monotonic):
        self.capacity = capacity
        self.ttl = ttl
        self.clock = clock
        self.data = {}

    def set(self, key, value, ttl=None):
        if len(self.data) >= self.capacity and key not in self.data:
            self.data.pop(next(iter(self.data)))
        self.data[key] = (value, self.clock() + (ttl or self.ttl))

    def get(self, key, default=None):
        item = self.data.get(key)
        if item is None:
            return default
        value, expires = item
        if self.clock() > expires:
            del self.data[key]
            return default
        return value

    def delete(self, key):
        return self.data.pop(key, None) is not None

    def __len__(self):
        return len(self.data)

    def keys(self):
        return list(self.data)
'''

TTL_SOLUTION = '''import time
from collections import OrderedDict


class TTLCache:
    def __init__(self, capacity, ttl, clock=time.monotonic):
        if capacity < 1 or ttl <= 0:
            raise ValueError("capacity must be >= 1 and ttl must be > 0")
        self.capacity = capacity
        self.ttl = ttl
        self.clock = clock
        self.data = OrderedDict()

    def _purge(self):
        now = self.clock()
        for key in [k for k, (_, expires) in self.data.items() if now >= expires]:
            del self.data[key]

    def set(self, key, value, ttl=None):
        if ttl is not None and ttl <= 0:
            raise ValueError("ttl must be > 0")
        self._purge()
        if key not in self.data and len(self.data) >= self.capacity:
            self.data.popitem(last=False)
        self.data[key] = (value, self.clock() + (self.ttl if ttl is None else ttl))
        self.data.move_to_end(key)

    def get(self, key, default=None):
        self._purge()
        if key not in self.data:
            return default
        value, expires = self.data[key]
        if SLIDING:
            expires = self.clock() + self.ttl
            self.data[key] = (value, expires)
        self.data.move_to_end(key)
        return value

    def delete(self, key):
        self._purge()
        return self.data.pop(key, None) is not None

    def __len__(self):
        self._purge()
        return len(self.data)

    def keys(self):
        self._purge()
        return list(self.data)
'''


def ttl_family(p: dict) -> dict:
    sliding = p["sliding"]
    rule_ko = ("get도 만료 시각을 지금 + 기본 ttl로 다시 잡는다." if sliding else "get은 사용 순서만 갱신하고 만료 시각은 바꾸지 않는다.")
    rule_mx = ("get 도 expiry 를 now + 기본 ttl 로 다시 잡는다." if sliding else "get 은 recency 만 갱신하고 expiry 는 바꾸지 않는다.")
    return dict(
        initial={"cache/__init__.py": "", "cache/ttl.py": TTL_INITIAL, "README.md": "# cache\n"},
        solution={"cache/ttl.py": TTL_SOLUTION.replace("SLIDING", str(sliding))},
        target="cache/ttl.py",
        cases=lambda rng: ttl_cases(p, rng),
        spec_ko=(
            "cache/ttl.py의 TTLCache를 고쳐 줘. 규칙: 항목은 now >= 만료 시각이면 만료다(경계 포함). set은 새 만료 시각(now + ttl, "
            "set의 ttl 인자가 있으면 그 값)과 사용 순서를 갱신한다. " + rule_ko + " 만료된 항목은 get, set, delete, len, keys 어디서든 먼저 치워서 "
            "없는 것으로 본다. 용량은 만료되지 않은 항목만 센다. 새 키를 넣을 때 용량이 차 있으면 가장 오래 쓰지 않은 항목을 하나 내보낸다. "
            "이미 있는 키를 덮어쓸 때는 아무것도 내보내지 않는다. keys()는 오래 쓰지 않은 것부터 최근 순서다. "
            "capacity가 1보다 작거나 ttl이 0 이하면 ValueError, set의 ttl 인자가 0 이하여도 ValueError."
        ),
        spec_mixed=(
            "cache/ttl.py 의 TTLCache 를 fix 해 줘. 규칙: item 은 now >= expiry 면 expired(boundary 포함). set 은 새 expiry(now + ttl, "
            "set 의 ttl 인자가 있으면 그 값)와 recency 를 update. " + rule_mx + " expired item 은 get, set, delete, len, keys 어디서든 먼저 purge 해서 "
            "없는 것으로 취급. capacity 는 만료 안 된 item 만 센다. 새 key 를 넣을 때 capacity 가 꽉 차 있으면 LRU item 하나를 evict. "
            "기존 key 를 overwrite 할 때는 evict 하지 않는다. keys() 는 LRU 부터 MRU 순. "
            "capacity 가 1 미만이거나 ttl 이 0 이하면 ValueError, set 의 ttl 인자가 0 이하여도 ValueError."
        ),
    )


def ttl_cases(p: dict, rng: random.Random):
    def make(seed, ops):
        def run(load, root):
            mod = load("cache/ttl.py")
            local = random.Random(seed)
            now = [100.0]
            try:
                cache = mod.TTLCache(3, 10, clock=lambda: now[0])
            except BaseException as error:  # noqa: BLE001
                return ("init-raise", type(error).__name__)
            trace = []
            for _ in range(ops):
                action = local.choice(["set", "set", "get", "get", "delete", "tick", "len", "keys", "setttl"])
                key = local.choice("abcde")
                try:
                    if action == "set":
                        cache.set(key, local.randint(0, 9))
                        trace.append("set")
                    elif action == "setttl":
                        cache.set(key, local.randint(0, 9), ttl=local.choice([1, 4, 10, 0]))
                        trace.append("setttl")
                    elif action == "get":
                        trace.append(("get", key, cache.get(key, "miss")))
                    elif action == "delete":
                        trace.append(("delete", key, cache.delete(key)))
                    elif action == "tick":
                        now[0] += local.choice([1, 3, 5, 10])
                        trace.append(("tick", now[0]))
                    elif action == "len":
                        trace.append(("len", len(cache)))
                    else:
                        trace.append(("keys", cache.keys()))
                except BaseException as error:  # noqa: BLE001
                    trace.append(("raise", type(error).__name__))
            return trace
        return run

    def bad_args(load, root):
        mod = load("cache/ttl.py")
        out = []
        for args in ((0, 5), (3, 0), (3, -1), (1, 1)):
            try:
                mod.TTLCache(*args)
                out.append("ok")
            except BaseException as error:  # noqa: BLE001
                out.append(type(error).__name__)
        return out

    cases = [(f"ttl-{i}", make(rng.randint(0, 10**6), 60)) for i in range(10)]
    cases.append(("ttl-args", bad_args))
    return cases


# ---------------------------------------------------------------- 평가 계열

DURATION_INITIAL = '''def parse_duration(text):
    """"1h30m" 같은 글을 초로 바꾼다."""
    total = 0
    number = ""
    for ch in text:
        if ch.isdigit():
            number += ch
        elif ch == "h":
            total += int(number) * 3600
            number = ""
        elif ch == "m":
            total += int(number) * 60
            number = ""
        elif ch == "s":
            total += int(number)
            number = ""
    return total
'''

DURATION_SOLUTION = '''import re
from decimal import ROUND_HALF_UP, Decimal

UNITS = UNITSDICT
_TOKEN = re.compile(r"(\\d+(?:\\.\\d{1,2})?)\\s*([a-z])")


def parse_duration(text):
    """"1h30m" 같은 글을 초로 바꾼다."""
    if not isinstance(text, str):
        raise ValueError("text must be a string")
    s = text.strip().lower()
    if not s:
        raise ValueError("empty")
    pos, last_rank, total, tokens = 0, -1, Decimal(0), []
    order = list(UNITS)
    while pos < len(s):
        m = _TOKEN.match(s, pos)
        if not m:
            raise ValueError("bad token")
        number, unit = m.group(1), m.group(2)
        if unit not in UNITS:
            raise ValueError("bad unit")
        rank = order.index(unit)
        if rank <= last_rank:
            raise ValueError("units out of order")
        last_rank = rank
        tokens.append((number, unit))
        total += Decimal(number) * UNITS[unit]
        pos = m.end()
        while pos < len(s) and s[pos] == " ":
            pos += 1
    for number, _ in tokens[:-1]:
        if "." in number:
            raise ValueError("only the last token may be fractional")
    return int(total.quantize(Decimal(1), rounding=ROUND_HALF_UP))
'''


def duration_family(p: dict) -> dict:
    units = {"w": 604800, "d": 86400, "h": 3600, "m": 60, "s": 1} if p["weeks"] else {"d": 86400, "h": 3600, "m": 60, "s": 1}
    unit_text = "".join(units)
    return dict(
        initial={"timeparse/__init__.py": "", "timeparse/duration.py": DURATION_INITIAL, "README.md": "# timeparse\n"},
        solution={"timeparse/duration.py": DURATION_SOLUTION.replace("UNITSDICT", repr(units))},
        target="timeparse/duration.py",
        cases=lambda rng: duration_cases(p, units, rng),
        spec_ko=(
            f"timeparse/duration.py의 parse_duration을 규칙대로 다시 써 줘. 입력은 '1h30m', '2h 15m', '90s' 같은 글이고 단위는 {unit_text}(대소문자 무시)다. "
            "앞뒤 공백은 무시하고 토큰 사이에는 공백이 있어도 된다. 단위는 위에 적은 순서(큰 단위부터)로만 나오고 같은 단위가 두 번 나오면 안 된다. "
            "마지막 토큰만 소수점 두 자리까지의 소수를 쓸 수 있다('1.5h'). 결과는 초 단위 int이고 정수가 아니면 반올림(0.5는 올림)한다. "
            "단위 없는 숫자, 빈 글, 모르는 단위, 음수, 순서 위반, 마지막이 아닌 토큰의 소수, 문자열이 아닌 입력은 모두 ValueError다. '0s'는 0이다."
        ),
        spec_mixed=(
            f"timeparse/duration.py 의 parse_duration 을 spec 대로 rewrite 해 줘. input 은 '1h30m', '2h 15m', '90s' 같은 string 이고 unit 은 {unit_text}(case-insensitive). "
            "앞뒤 whitespace 는 trim, token 사이 space 는 허용. unit 은 위 순서(큰 unit 부터)로만 나오고 같은 unit 이 두 번 나오면 invalid. "
            "마지막 token 만 소수점 두 자리까지 decimal 허용('1.5h'). 결과는 seconds 단위 int, 정수가 아니면 round half up. "
            "unit 없는 number, empty string, unknown unit, negative, order 위반, 마지막이 아닌 token 의 decimal, non-string input 은 모두 ValueError. '0s' 는 0."
        ),
    )


def duration_cases(p: dict, units: dict, rng: random.Random):
    texts = ["1h30m", "2h 15m", "90s", "0s", " 5m ", "1H", "1.5h", "1.25m", "0.5s", "1.05s", "1.5h30m", "30m1h", "1h1h", "90", "", " ",
             "-1h", "1x", "h", "1 h", "1h 30 m", "2d4h", "1d1.5h", "1w", "1w1d", "10m30", "1.555s", "1.m", "3h0m0s", "00s"]
    allowed = list(units)
    for _ in range(12):
        picked = sorted(rng.sample(allowed, rng.randint(1, len(allowed))), key=allowed.index)
        parts = [f"{rng.randint(0, 99)}{u}" for u in picked]
        texts.append(rng.choice(["", " "]).join(parts))

    def make(text):
        def run(load, root):
            mod = load("timeparse/duration.py")
            try:
                return mod.parse_duration(text)
            except BaseException as error:  # noqa: BLE001
                return ("raise", type(error).__name__)
        return run

    cases = [(f"dur-{i}", make(t)) for i, t in enumerate(texts)]
    cases.append(("dur-nonstring", make(None)))
    return cases


RANGES_INITIAL = '''def merge_ranges(ranges, gap=0):
    """겹치는 구간을 합친다."""
    result = []
    for start, end in ranges:
        if result and start < result[-1][1]:
            result[-1] = (result[-1][0], max(result[-1][1], end))
        else:
            result.append((start, end))
    return result


def total_coverage(ranges, gap=0):
    return sum(end - start for start, end in merge_ranges(ranges, gap))
'''

RANGES_SOLUTION = '''def merge_ranges(ranges, gap=GAPDEFAULT):
    """겹치는 구간을 합친다."""
    if gap < 0:
        raise ValueError("gap must not be negative")
    items = []
    for start, end in ranges:
        if start > end:
            raise ValueError("start must not exceed end")
        items.append((start, end))
    items.sort()
    result = []
    for start, end in items:
        if result and start <= result[-1][1] + gap:
            result[-1] = (result[-1][0], max(result[-1][1], end))
        else:
            result.append((start, end))
    return result


def total_coverage(ranges, gap=GAPDEFAULT):
    return sum(end - start for start, end in merge_ranges(ranges, gap))
'''


def ranges_family(p: dict) -> dict:
    g = p["gap"]
    return dict(
        initial={"spans/__init__.py": "", "spans/merge.py": RANGES_INITIAL, "README.md": "# spans\n"},
        solution={"spans/merge.py": RANGES_SOLUTION.replace("GAPDEFAULT", str(g))},
        target="spans/merge.py",
        cases=lambda rng: ranges_cases(p, rng),
        spec_ko=(
            f"spans/merge.py의 merge_ranges와 total_coverage를 고쳐 줘. gap의 기본값은 {g}이다. 입력은 (start, end) 쌍의 목록이고 정렬돼 있지 않을 수 있다. "
            "다음 구간의 start가 지금까지 합친 구간의 end + gap 이하이면 합친다(맞닿는 구간도 합친다). 결과는 start 오름차순의 튜플 리스트다. "
            "입력 목록과 그 안의 원소를 바꾸면 안 된다. start가 end보다 큰 쌍이 있거나 gap이 음수면 ValueError. 빈 입력은 빈 리스트다. "
            "total_coverage는 합친 구간들의 길이(end - start) 합이다."
        ),
        spec_mixed=(
            f"spans/merge.py 의 merge_ranges 와 total_coverage 를 fix 해 줘. gap 의 default 는 {g}. input 은 (start, end) pair 의 list 이고 unsorted 일 수 있다. "
            "다음 range 의 start 가 지금까지 merge 한 range 의 end + gap 이하면 merge(touching 도 merge). 결과는 start ascending 의 tuple list. "
            "input list 와 그 원소를 mutate 하면 안 된다. start 가 end 보다 큰 pair 가 있거나 gap 이 negative 면 ValueError. empty input 은 empty list. "
            "total_coverage 는 merge 된 range 들의 length(end - start) 합."
        ),
    )


def ranges_cases(p: dict, rng: random.Random):
    inputs = [[], [(1, 2)], [(1, 3), (2, 5)], [(1, 3), (3, 5)], [(1, 3), (4, 5)], [(5, 6), (1, 2), (2, 4)], [(1, 10), (2, 3)],
              [(3, 1)], [(0, 0), (0, 0)], [(1, 2), (6, 7), (3, 5)], [[1, 2], [2, 3]]]
    for _ in range(8):
        inputs.append([(a, a + rng.randint(0, 6)) for a in (rng.randint(0, 40) for _ in range(rng.randint(0, 9)))])
    cases = []

    def make(data, gap):
        def run(load, root):
            mod = load("spans/merge.py")
            import copy
            snapshot = copy.deepcopy(data)
            try:
                kwargs = {} if gap is None else {"gap": gap}
                merged = mod.merge_ranges(data, **kwargs)
                coverage = mod.total_coverage(data, **kwargs)
                return merged, coverage, data == snapshot, [type(x).__name__ for x in merged]
            except BaseException as error:  # noqa: BLE001
                return ("raise", type(error).__name__)
        return run

    for i, data in enumerate(inputs):
        for gap in (None, 0, 2):
            cases.append((f"rng-{i}-{gap}", make(data, gap)))
    cases.append(("rng-negative-gap", make([(1, 2)], -1)))
    return cases


CART_INIT = {
    "shop/__init__.py": "",
    "shop/cart.py": '''from decimal import Decimal


class Line:
    def __init__(self, sku, price, qty, taxable=True):
        self.sku = sku
        self.price = Decimal(str(price))
        self.qty = qty
        self.taxable = taxable

    def amount(self):
        return round(self.price * self.qty, 2)
''',
    "shop/coupons.py": '''from decimal import Decimal


def discount_for(coupon, subtotal):
    """coupon은 None, ("percent", n), ("fixed", amount) 중 하나다."""
    if coupon is None:
        return Decimal("0.00")
    kind, value = coupon
    if kind == "percent":
        return round(subtotal * Decimal(str(value)) / 100, 2)
    return Decimal(str(value))
''',
    "shop/tax.py": '''from decimal import Decimal

RATES = {"KR": Decimal("0.10"), "US": Decimal("0.07"), "DE": Decimal("0.19")}


def tax_for(base, region):
    return round(base * RATES[region], 2)
''',
    "shop/shipping.py": '''from decimal import Decimal

FREE_THRESHOLD = Decimal("@FREE@")
FLAT_FEE = Decimal("@FEE@")


def shipping_for(subtotal):
    return Decimal("0.00") if subtotal >= FREE_THRESHOLD else FLAT_FEE
''',
    "shop/checkout.py": '''from decimal import Decimal

from .coupons import discount_for
from .shipping import shipping_for
from .tax import tax_for


def compute_total(lines, region, coupon=None):
    """주문 합계를 계산한다. 값은 모두 Decimal이다."""
    subtotal = sum((line.amount() for line in lines), Decimal("0.00"))
    tax = tax_for(subtotal, region)
    discount = discount_for(coupon, subtotal)
    discounted = subtotal - discount
    shipping = shipping_for(subtotal)
    return {
        "subtotal": subtotal,
        "discount": discount,
        "tax": tax,
        "shipping": shipping,
        "total": discounted + tax + shipping,
    }
''',
}

CART_SOLUTION = {
    "shop/cart.py": '''from decimal import ROUND_HALF_UP, Decimal


def money(value):
    return Decimal(value).quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)


class Line:
    def __init__(self, sku, price, qty, taxable=True):
        self.sku = sku
        self.price = Decimal(str(price))
        self.qty = qty
        self.taxable = taxable

    def amount(self):
        return money(self.price * self.qty)
''',
    "shop/coupons.py": '''from decimal import Decimal

from .cart import money


def discount_for(coupon, subtotal):
    """coupon은 None, ("percent", n), ("fixed", amount) 중 하나다."""
    if coupon is None:
        return Decimal("0.00")
    kind, value = coupon
    if kind == "percent":
        return money(subtotal * Decimal(str(value)) / 100)
    return min(money(Decimal(str(value))), subtotal)
''',
    "shop/tax.py": '''from decimal import Decimal

from .cart import money

RATES = {"KR": Decimal("0.10"), "US": Decimal("0.07"), "DE": Decimal("0.19")}


def tax_for(base, region):
    return money(base * RATES[region])
''',
    "shop/shipping.py": '''from decimal import Decimal

FREE_THRESHOLD = Decimal("@FREE@")
FLAT_FEE = Decimal("@FEE@")


def shipping_for(subtotal):
    return Decimal("0.00") if subtotal >= FREE_THRESHOLD else FLAT_FEE
''',
    "shop/checkout.py": '''from decimal import Decimal

from .cart import money
from .coupons import discount_for
from .shipping import shipping_for
from .tax import tax_for


def compute_total(lines, region, coupon=None):
    """주문 합계를 계산한다. 값은 모두 Decimal이다."""
    subtotal = sum((line.amount() for line in lines), Decimal("0.00"))
    taxable = sum((line.amount() for line in lines if line.taxable), Decimal("0.00"))
    discount = discount_for(coupon, subtotal)
    discounted = subtotal - discount
    if subtotal > 0:
        taxable_after = taxable - money(discount * taxable / subtotal)
    else:
        taxable_after = Decimal("0.00")
    tax = tax_for(taxable_after, region)
    shipping = shipping_for(discounted)
    return {
        "subtotal": subtotal,
        "discount": discount,
        "tax": tax,
        "shipping": shipping,
        "total": discounted + tax + shipping,
    }
''',
}


def cart_family(p: dict) -> dict:
    sub = lambda text: text.replace("@FREE@", p["free"]).replace("@FEE@", p["fee"])  # noqa: E731
    initial = {k: (sub(v) if "shipping" in k else v) for k, v in CART_INIT.items()}
    solution = {k: (sub(v) if "shipping" in k else v) for k, v in CART_SOLUTION.items()}
    initial["README.md"] = "# shop\n\n주문 합계 계산. 진입점은 shop/checkout.py의 compute_total.\n"
    return dict(
        initial=initial,
        solution=solution,
        target="shop/checkout.py",
        cases=lambda rng: cart_cases(p, rng),
        spec_ko=(
            "shop 패키지의 주문 합계 계산(compute_total)이 규칙과 다르게 나온다. 규칙에 맞게 고쳐 줘. 규칙: (1) 모든 금액 반올림은 0.01 단위 half-up이다(파이썬 round 쓰지 말 것). "
            "줄 금액은 줄마다 반올림한다. (2) 쿠폰은 세금 전에 적용한다. percent는 합계에 비율을 곱해 반올림, fixed는 합계를 넘을 수 없다. "
            "(3) 세금은 쿠폰 적용 뒤 금액에 매긴다. 세금 없는 줄(taxable=False)이 있으면 할인은 줄 금액에 비례해 나눠 받는다고 보고, "
            "과세 금액 = 과세 줄 합계 - round_half_up(할인 * 과세 줄 합계 / 합계)로 계산한 뒤 세율을 곱해 반올림한다. "
            f"(4) 배송비는 할인 뒤 금액이 {p['free']} 이상이면 0, 아니면 {p['fee']}이고 세금 대상이 아니다. (5) total = 할인 뒤 금액 + 세금 + 배송비. "
            "반환 dict의 키와 Decimal 타입은 그대로 둔다."
        ),
        spec_mixed=(
            "shop package 의 compute_total 결과가 rule 과 다르게 나온다. rule 에 맞게 fix 해 줘. rule: (1) 모든 반올림은 0.01 단위 half-up(python round 쓰지 말 것). "
            "line amount 는 line 마다 round. (2) coupon 은 tax 전에 apply. percent 는 subtotal 에 비율을 곱해 round, fixed 는 subtotal 을 넘을 수 없다. "
            "(3) tax 는 coupon 적용 후 금액에 부과. taxable=False 인 line 이 있으면 discount 는 line amount 에 비례 배분된다고 보고, "
            "taxable base = taxable line 합계 - round_half_up(discount * taxable 합계 / subtotal) 로 계산한 뒤 rate 를 곱해 round. "
            f"(4) shipping 은 discount 후 금액이 {p['free']} 이상이면 0, 아니면 {p['fee']}, tax 대상 아님. (5) total = discounted + tax + shipping. "
            "반환 dict 의 key 와 Decimal type 은 그대로."
        ),
    )


def cart_cases(p: dict, rng: random.Random):
    prices = ["0.05", "0.99", "1.005", "2.50", "9.99", "12.345", "19.90", "33.33", "49.995", "120.00"]
    coupons = [None, ("percent", 10), ("percent", 33), ("percent", 100), ("fixed", 5), ("fixed", 500), ("percent", "12.5")]
    cases = []

    def make(spec, region, coupon):
        def run(load, root):
            cart = load("shop/cart.py")
            mod = load("shop/checkout.py")
            lines = [cart.Line(f"sku{i}", price, qty, taxable) for i, (price, qty, taxable) in enumerate(spec)]
            try:
                return {k: str(v) for k, v in mod.compute_total(lines, region, coupon).items()}
            except BaseException as error:  # noqa: BLE001
                return ("raise", type(error).__name__)
        return run

    for i in range(22):
        spec = [(rng.choice(prices), rng.randint(1, 4), rng.random() < 0.7) for _ in range(rng.randint(1, 4))]
        cases.append((f"cart-{i}", make(spec, rng.choice(["KR", "US", "DE"]), rng.choice(coupons))))
    cases.append(("cart-empty", make([], "KR", None)))
    cases.append(("cart-exempt-only", make([("10.00", 3, False)], "DE", ("percent", 20))))
    return cases


CONFIG_FILES = {
    "settings.py": '''"""앱 설정."""

DEFAULT_TIMEOUT = TO_OLD
MAX_RETRIES = RE_OLD
ENVIRONMENTS = ["dev", "prod"]
PAGE_SIZE = PS_OLD
''',
    "docs/config.md": """# 설정

| 키 | 값 | 설명 |
|---|---|---|
| timeout | TO_OLD | 요청 제한 시간(초) |
| retries | RE_OLD | 최대 재시도 횟수 |
| page_size | PS_OLD | 한 쪽 항목 수 |

환경: dev, prod
""",
    "client.py": '''from settings import DEFAULT_TIMEOUT, MAX_RETRIES


def request_plan():
    return {"timeout": DEFAULT_TIMEOUT, "retries": MAX_RETRIES}
''',
}


def config_family(p: dict) -> dict:
    def fill(text, to, re_, ps):
        return text.replace("TO_OLD", str(to)).replace("RE_OLD", str(re_)).replace("PS_OLD", str(ps))

    initial = {k: fill(v, p["to_old"], p["re_old"], p["ps_old"]) for k, v in CONFIG_FILES.items()}
    solution = {
        "settings.py": fill(CONFIG_FILES["settings.py"], p["to_new"], p["re_new"], p["ps_old"]).replace('["dev", "prod"]', '["dev", "staging", "prod"]'),
        "docs/config.md": fill(CONFIG_FILES["docs/config.md"], p["to_new"], p["re_new"], p["ps_old"]).replace("환경: dev, prod", "환경: dev, staging, prod"),
    }
    return dict(
        initial=initial,
        solution=solution,
        target="settings.py",
        cases=lambda rng: config_cases(p),
        spec_ko=(
            f"settings.py에서 DEFAULT_TIMEOUT을 {p['to_new']}으로, MAX_RETRIES를 {p['re_new']}로 바꾸고 ENVIRONMENTS에 'staging'을 dev와 prod 사이에 넣어 줘. "
            "docs/config.md의 표와 환경 줄도 같은 값으로 맞춰 줘. 다른 설정은 건드리지 마."
        ),
        spec_mixed=(
            f"settings.py 에서 DEFAULT_TIMEOUT 을 {p['to_new']} 으로, MAX_RETRIES 를 {p['re_new']} 로 바꾸고 ENVIRONMENTS 에 'staging' 을 dev 와 prod 사이에 추가해 줘. "
            "docs/config.md 의 table 과 환경 line 도 같은 값으로 sync 해 줘. 다른 setting 은 건드리지 마."
        ),
    )


def config_cases(p: dict):
    def values(load, root):
        mod = load("settings.py")
        client = load("client.py")
        return mod.DEFAULT_TIMEOUT, mod.MAX_RETRIES, mod.ENVIRONMENTS, mod.PAGE_SIZE, client.request_plan()

    def docs(load, root):
        return (root / "docs/config.md").read_text(encoding="utf-8").strip()

    return [("config-values", values), ("config-docs", docs)]


CSV_INITIAL = '''def summarize(text):
    """판매 CSV를 지역별로 합산한다."""
    totals = {}
    for line in text.splitlines():
        parts = line.split(",")
        if parts[0] == "region":
            continue
        totals[parts[0]] = totals.get(parts[0], 0) + float(parts[1])
    return sorted(totals.items(), key=lambda item: item[1], reverse=True), 0
'''

CSV_SOLUTION = '''from decimal import ROUND_HALF_UP, Decimal, InvalidOperation


def summarize(text):
    """판매 CSV를 지역별로 합산한다. (결과, 건너뛴 줄 수)를 돌려준다."""
    totals, names, skipped, header = {}, {}, 0, False
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = [part.strip() for part in line.split(",")]
        if not header:
            header = True
            if [part.lower() for part in parts[:2]] == ["region", "amount"]:
                continue
        if len(parts) not in (2, 3) or not parts[0]:
            skipped += 1
            continue
        try:
            amount = Decimal(parts[1])
        except InvalidOperation:
            skipped += 1
            continue
        kind = parts[2].lower() if len(parts) == 3 else "sale"
        if kind not in ("sale", "refund") or not amount.is_finite():
            skipped += 1
            continue
        if kind == "refund":
            amount = -abs(amount)
        key = parts[0].casefold()
        names.setdefault(key, parts[0])
        totals[key] = totals.get(key, Decimal(0)) + amount
    rows = [(names[key], str(total.quantize(Decimal("0.01"), rounding=ROUND_HALF_UP))) for key, total in totals.items()]
    rows.sort(key=lambda row: (-Decimal(row[1]), row[0].casefold()))
    return rows, skipped
'''


def csv_family(p: dict) -> dict:
    return dict(
        initial={"report/__init__.py": "", "report/sales.py": CSV_INITIAL, "README.md": "# report\n"},
        solution={"report/sales.py": CSV_SOLUTION},
        target="report/sales.py",
        cases=lambda rng: csv_cases(p, rng),
        spec_ko=(
            "report/sales.py의 summarize를 규칙대로 고쳐 줘. 입력은 CSV 글이다. 비어 있거나 #로 시작하는 줄은 무시한다. 헤더('region,amount' 로 시작하는 첫 의미 있는 줄)는 있을 수도 없을 수도 있다. "
            "각 줄은 region,amount[,type]이고 type은 sale(기본) 또는 refund다(대소문자 무시). refund는 amount의 부호와 관계없이 합계에서 뺀다. "
            "열 개수가 2나 3이 아니거나 region이 비었거나 amount가 숫자가 아니거나 type이 모르는 값이면 그 줄은 건너뛰고 센다. "
            "지역은 앞뒤 공백을 지우고 대소문자 구분 없이 묶으며 출력 이름은 처음 나온 표기를 쓴다. 금액은 Decimal로 더하고 0.01 단위 half-up으로 반올림해 문자열로 낸다. "
            "반환은 (행 목록, 건너뛴 줄 수)이고 행은 (지역, 합계 문자열)이며 합계 내림차순, 같으면 지역 이름(대소문자 무시) 오름차순이다."
        ),
        spec_mixed=(
            "report/sales.py 의 summarize 를 rule 대로 fix 해 줘. input 은 CSV text. 빈 줄이나 # 로 시작하는 line 은 ignore. header('region,amount' 로 시작하는 첫 의미 있는 line)는 있을 수도 없을 수도 있다. "
            "각 line 은 region,amount[,type] 이고 type 은 sale(default) 또는 refund(case-insensitive). refund 는 amount 의 sign 과 무관하게 합계에서 뺀다. "
            "column 수가 2 나 3 이 아니거나 region 이 empty 거나 amount 가 numeric 이 아니거나 type 이 unknown 이면 그 line 은 skip 하고 count 한다. "
            "region 은 trim 하고 case-insensitive 로 group, 출력 이름은 first-seen 표기. 금액은 Decimal 로 더하고 0.01 단위 half-up round 후 string 으로 낸다. "
            "return 은 (rows, skipped count), row 는 (region, total string), total 내림차순 후 region(case-insensitive) 오름차순."
        ),
    )


def csv_cases(p: dict, rng: random.Random):
    regions = ["Seoul", "seoul", "Busan", " Daegu ", "busan", "Jeju", "", "Ulsan"]
    kinds = ["", ",sale", ",refund", ",REFUND", ",gift", ",sale,extra"]
    amounts = ["10", "10.005", "-3.50", "abc", "0.1", "0.2", "1e2", "nan", "7.995", "", "100.00"]
    texts = ["", "region,amount\n", "region,amount,type\nSeoul,10\n", "# c\n\nSeoul,5\nSEOUL,5.005\nBusan,10,refund\nBusan,-4,refund\n"]
    for _ in range(14):
        lines = []
        if rng.random() < 0.5:
            lines.append(rng.choice(["region,amount", "Region,Amount,Type", "# note", ""]))
        for _ in range(rng.randint(1, 9)):
            lines.append(f"{rng.choice(regions)},{rng.choice(amounts)}{rng.choice(kinds)}")
        texts.append("\n".join(lines))

    def make(text):
        def run(load, root):
            mod = load("report/sales.py")
            try:
                rows, skipped = mod.summarize(text)
                return [(name, str(total)) for name, total in rows], skipped
            except BaseException as error:  # noqa: BLE001
                return ("raise", type(error).__name__)
        return run

    return [(f"csv-{i}", make(t)) for i, t in enumerate(texts)]


SEMVER_INITIAL = '''def compare(a, b):
    """두 버전을 비교해 -1, 0, 1을 돌려준다."""
    pa = [int(x) for x in a.split(".")]
    pb = [int(x) for x in b.split(".")]
    return (pa > pb) - (pa < pb)


def sort_versions(versions, reverse=False):
    return sorted(versions, key=lambda v: [int(x) for x in v.split(".")], reverse=reverse)


def satisfies(version, rule):
    return compare(version, rule.lstrip("^~>=")) >= 0
'''

SEMVER_SOLUTION = '''import functools
import re

_VERSION = re.compile(
    r"^(0|[1-9]\\d*)\\.(0|[1-9]\\d*)\\.(0|[1-9]\\d*)(?:-([0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*))?(?:\\+[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*)?$"
)
INVALID = "INVALIDMODE"


def parse(version):
    if not isinstance(version, str):
        raise ValueError("version must be a string")
    m = _VERSION.match(version)
    if not m:
        raise ValueError("invalid version")
    pre = m.group(4)
    ids = None
    if pre is not None:
        ids = pre.split(".")
        for ident in ids:
            if ident.isdigit() and len(ident) > 1 and ident.startswith("0"):
                raise ValueError("leading zero in prerelease")
    return (int(m.group(1)), int(m.group(2)), int(m.group(3)), ids)


def _cmp_ids(x, y):
    for a, b in zip(x, y):
        if a == b:
            continue
        if a.isdigit() and b.isdigit():
            return -1 if int(a) < int(b) else 1
        if a.isdigit():
            return -1
        if b.isdigit():
            return 1
        return -1 if a < b else 1
    return (len(x) > len(y)) - (len(x) < len(y))


def _key_cmp(pa, pb):
    if pa[:3] != pb[:3]:
        return -1 if pa[:3] < pb[:3] else 1
    if pa[3] is None and pb[3] is None:
        return 0
    if pa[3] is None:
        return 1
    if pb[3] is None:
        return -1
    return _cmp_ids(pa[3], pb[3])


def compare(a, b):
    """두 버전을 비교해 -1, 0, 1을 돌려준다."""
    return _key_cmp(parse(a), parse(b))


def sort_versions(versions, reverse=False):
    good = []
    for v in versions:
        try:
            parse(v)
        except ValueError:
            if INVALID == "raise":
                raise
            continue
        good.append(v)
    return sorted(good, key=functools.cmp_to_key(compare), reverse=reverse)


def satisfies(version, rule):
    v = parse(version)
    rule = rule.strip()
    if rule.startswith(">="):
        base = parse(rule[2:].strip())
        op = ">="
    elif rule.startswith("^"):
        base = parse(rule[1:].strip())
        op = "^"
    elif rule.startswith("~"):
        base = parse(rule[1:].strip())
        op = "~"
    else:
        base = parse(rule)
        op = "="
    if v[3] is not None and not (base[3] is not None and base[:3] == v[:3]):
        return False
    if op == "=":
        return _key_cmp(v, base) == 0
    if _key_cmp(v, base) < 0:
        return False
    if op == ">=":
        return True
    major, minor, patch = base[:3]
    if op == "~":
        upper = (major, minor + 1, 0)
    elif major > 0:
        upper = (major + 1, 0, 0)
    elif minor > 0:
        upper = (0, minor + 1, 0)
    else:
        upper = (0, 0, patch + 1)
    return v[:3] < upper
'''


def semver_family(p: dict) -> dict:
    mode = p["invalid"]
    mode_ko = "유효하지 않은 버전이 섞여 있으면 ValueError를 낸다." if mode == "raise" else "유효하지 않은 버전은 조용히 빼고 나머지를 정렬한다."
    mode_mx = "invalid version 이 섞여 있으면 ValueError 를 낸다." if mode == "raise" else "invalid version 은 조용히 drop 하고 나머지를 sort 한다."
    return dict(
        initial={"semver/__init__.py": "", "semver/core.py": SEMVER_INITIAL, "README.md": "# semver\n"},
        solution={"semver/core.py": SEMVER_SOLUTION.replace("INVALIDMODE", mode)},
        target="semver/core.py",
        cases=lambda rng: semver_cases(p, rng),
        spec_ko=(
            "semver/core.py의 compare, sort_versions, satisfies를 SemVer 2.0.0 규칙대로 고쳐 줘. 버전은 MAJOR.MINOR.PATCH[-사전배포][+빌드]이고 숫자 앞 0은 안 된다(사전배포의 숫자 식별자도 같다). "
            "형식이 틀리면 compare와 satisfies는 ValueError. 비교는 숫자 순, 사전배포가 있으면 같은 번호의 정식 버전보다 낮고, 사전배포끼리는 점으로 나눈 식별자를 앞에서부터 비교한다"
            "(숫자끼리는 숫자 크기, 숫자 식별자는 글자 식별자보다 낮고, 글자끼리는 사전순, 앞부분이 같으면 식별자가 많은 쪽이 높다). 빌드 메타데이터는 비교에서 무시한다. "
            "sort_versions는 오름차순(reverse면 내림차순)이고 " + mode_ko + " satisfies의 규칙은 '>=X', '^X', '~X', 정확히 X 넷이다. "
            "^X는 >=X이면서 X의 0이 아닌 첫 자리가 오르지 않는 범위(^1.2.3은 <2.0.0, ^0.2.3은 <0.3.0, ^0.0.3은 <0.0.4), ~X는 >=X이면서 <MAJOR.(MINOR+1).0이다. "
            "사전배포 버전은 규칙의 버전이 같은 MAJOR.MINOR.PATCH의 사전배포일 때만 만족할 수 있다."
        ),
        spec_mixed=(
            "semver/core.py 의 compare, sort_versions, satisfies 를 SemVer 2.0.0 규칙대로 fix 해 줘. version 은 MAJOR.MINOR.PATCH[-prerelease][+build] 이고 numeric 앞 0 은 안 된다(prerelease 의 numeric identifier 도 동일). "
            "format 이 틀리면 compare 와 satisfies 는 ValueError. 비교는 numeric 순, prerelease 가 있으면 같은 번호의 release 보다 낮고, prerelease 끼리는 dot 으로 나눈 identifier 를 앞에서부터 비교"
            "(numeric 끼리는 숫자 크기, numeric 은 alphanumeric 보다 낮고, alphanumeric 끼리는 lexical, 앞부분이 같으면 identifier 가 많은 쪽이 높다). build metadata 는 비교에서 무시. "
            "sort_versions 는 ascending(reverse 면 descending)이고 " + mode_mx + " satisfies 의 rule 은 '>=X', '^X', '~X', exact X 넷이다. "
            "^X 는 >=X 이면서 X 의 0 이 아닌 첫 자리가 오르지 않는 범위(^1.2.3 은 <2.0.0, ^0.2.3 은 <0.3.0, ^0.0.3 은 <0.0.4), ~X 는 >=X 이면서 <MAJOR.(MINOR+1).0. "
            "prerelease version 은 rule 의 version 이 같은 MAJOR.MINOR.PATCH 의 prerelease 일 때만 만족할 수 있다."
        ),
    )


def semver_cases(p: dict, rng: random.Random):
    versions = ["1.0.0", "1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1",
                "2.0.0", "2.1.0", "2.1.1", "0.9.9", "0.0.3", "0.2.3", "0.2.9", "1.2.3+build.5", "1.2.3", "01.2.3", "1.2", "1.2.3-01",
                "1.0.0-alpha.0", "1.0.0-x.7.z.92", "v1.0.0", "1.10.0", "1.9.0"]
    rules = [">=1.0.0", ">=1.0.0-beta", "^1.2.3", "^0.2.3", "^0.0.3", "~1.2.3", "~1.0.0", "1.0.0", "1.0.0-beta", "^1.0.0-alpha", ">=x", "^1.2", "~2.1.0"]
    cases = []

    def cmp_case(a, b):
        def run(load, root):
            mod = load("semver/core.py")
            try:
                return mod.compare(a, b)
            except BaseException as error:  # noqa: BLE001
                return ("raise", type(error).__name__)
        return run

    pairs = [(rng.choice(versions), rng.choice(versions)) for _ in range(40)]
    pairs += [("1.0.0", "1.0.0-rc.1"), ("1.0.0-alpha", "1.0.0-alpha.1"), ("1.0.0-beta.2", "1.0.0-beta.11"), ("1.2.3+a", "1.2.3+b")]
    for i, (a, b) in enumerate(pairs):
        cases.append((f"cmp-{i}", cmp_case(a, b)))

    def sat_case(v, r):
        def run(load, root):
            mod = load("semver/core.py")
            try:
                return mod.satisfies(v, r)
            except BaseException as error:  # noqa: BLE001
                return ("raise", type(error).__name__)
        return run

    for i in range(45):
        cases.append((f"sat-{i}", sat_case(rng.choice(versions), rng.choice(rules))))

    def sort_case(items, reverse):
        def run(load, root):
            mod = load("semver/core.py")
            try:
                return mod.sort_versions(items, reverse=reverse)
            except BaseException as error:  # noqa: BLE001
                return ("raise", type(error).__name__)
        return run

    valid = [v for v in versions if v not in ("01.2.3", "1.2", "1.2.3-01", "v1.0.0")]
    cases.append(("sort-valid", sort_case(valid, False)))
    cases.append(("sort-valid-rev", sort_case(valid, True)))
    cases.append(("sort-mixed", sort_case(versions, False)))
    return cases


# ---------------------------------------------------------------- 표

FAMILIES = {
    # 개발
    "retry": dict(build=retry_family, difficulty="medium", split="dev",
                  params=lambda v: dict(factor=[2, 3, 2, 3][v % 4], cap=[8, 10, 20, 6][v % 4])),
    "paginate": dict(build=paginate_family, difficulty="medium", split="dev",
                     params=lambda v: dict(max_per_page=[50, 100, 25, 40][v % 4])),
    "ttl": dict(build=ttl_family, difficulty="hard", split="dev",
                params=lambda v: dict(sliding=[False, True, True, False][v % 4])),
    # 평가
    "duration": dict(build=duration_family, difficulty="hard", split="confirm",
                     params=lambda v: dict(weeks=bool(v % 2))),
    "ranges": dict(build=ranges_family, difficulty="medium", split="confirm",
                   params=lambda v: dict(gap=[0, 1, 5, 2, 0, 3][v % 6])),
    "cart": dict(build=cart_family, difficulty="hard", split="confirm",
                 params=lambda v: dict(free=["50.00", "75.00", "60.00", "40.00", "100.00", "80.00"][v % 6], fee=["4.99", "5.99", "3.50", "2.99", "6.00", "4.50"][v % 6])),
    "config": dict(build=config_family, difficulty="easy", split="confirm",
                   params=lambda v: dict(to_old=[30, 15, 20, 45, 60, 25][v % 6], to_new=[60, 45, 40, 90, 120, 50][v % 6], re_old=[3, 2, 4, 5, 1, 3][v % 6],
                                         re_new=[5, 4, 6, 8, 3, 7][v % 6], ps_old=[20, 50, 10, 25, 30, 15][v % 6])),
    "csv": dict(build=csv_family, difficulty="medium", split="confirm", params=lambda v: {}),
    "semver": dict(build=semver_family, difficulty="hard", split="confirm",
                   params=lambda v: dict(invalid=["raise", "skip"][v % 2])),
}
