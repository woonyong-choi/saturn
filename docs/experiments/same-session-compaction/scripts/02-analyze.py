"""Recompute paired compaction outcomes from local raw JSONL."""
import hashlib
import json
import random
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
RAW = ROOT / '.local/experiments/same-session-compaction/formal/results.jsonl'
OUT = Path(__file__).resolve().parents[1] / 'results/summary.json'


def paired(rows, a, b, seed):
    pairs = [(int(r['arms'][a]['exact']), int(r['arms'][b]['exact'])) for r in rows]
    n = len(pairs)
    gap = sum(x - y for x, y in pairs) / n if n else None
    rng = random.Random(seed)
    # Resample paired rows with the same index for both arms.
    if n:
        rng = random.Random(seed)
        samples = sorted(sum((lambda p: p[0] - p[1])(pairs[rng.randrange(n)])
                             for _ in range(n)) / n for _ in range(10000))
    return {'n': n, 'difference': gap,
            'ci95': [samples[249], samples[9749]] if samples else None,
            'a_only': sum(x and not y for x, y in pairs),
            'b_only': sum(y and not x for x, y in pairs)}


def main():
    if not RAW.exists():
        raise SystemExit('raw data missing')
    raw = RAW.read_bytes()
    if b'apikey_' in raw:
        raise SystemExit('secret-like string in raw data')
    rows = [json.loads(line) for line in raw.decode().splitlines() if line.strip()]
    if len({r['index'] for r in rows}) != len(rows):
        raise SystemExit('duplicate scenario index')
    ok = [r for r in rows if 'error' not in r]
    summary = {'attempted': len(rows), 'complete': len(ok),
               'errors': [{'index': r['index'], 'error': r['error']} for r in rows if 'error' in r],
               'raw_sha256': hashlib.sha256(raw).hexdigest(),
               'conditions': {}, 'paired': {}, 'hook_applied': 0}
    for arm in ['full', 'native', 'fast', 'packet']:
        hits = sum(bool(r['arms'][arm]['exact']) for r in ok)
        summary['conditions'][arm] = {'hits': hits, 'n': len(ok),
                                       'rate': hits / len(ok) if ok else None}
    summary['hook_applied'] = sum(r['arms']['fast']['hook_applied'] for r in ok)
    summary['paired']['fast_minus_native'] = paired(ok, 'fast', 'native', 20261005)
    summary['paired']['packet_minus_full'] = paired(ok, 'packet', 'full', 20261006)
    summary['packet_tokens'] = [r['arms']['packet']['packet_tokens'] for r in ok]
    OUT.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + '\n'
    if '--verify' in sys.argv and OUT.exists() and OUT.read_text() != text:
        raise SystemExit('summary differs from recomputation')
    OUT.write_text(text)
    print(json.dumps({'attempted': len(rows), 'complete': len(ok),
                      'hits': {k: v['hits'] for k, v in summary['conditions'].items()},
                      'hook_applied': summary['hook_applied']}, ensure_ascii=False))


if __name__ == '__main__':
    main()
