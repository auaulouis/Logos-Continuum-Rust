import json
import hashlib
import re
from pathlib import Path
from collections import Counter, defaultdict

ROOT = Path('/Users/louis/Logos Rust')
BENCH = ROOT / 'tmp' / 'bench-compare'
PY_INDEX = BENCH / 'python_local_docs' / 'cards_index.json'
RS_INDEX = BENCH / 'rust_local_docs' / 'cards_index.json'
OUT = BENCH / 'index_diff_report.json'


def read_jsonl(path: Path):
    rows = []
    with path.open('r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(obj, dict):
                rows.append(obj)
    return rows


def normalize_ws(s: str) -> str:
    return ' '.join(str(s or '').split())


CID_RE = re.compile(r"\s*\[\[CID-[^\]]+\]\]\s*", re.IGNORECASE)


def strip_cid_tokens(s: str) -> str:
    return normalize_ws(CID_RE.sub(' ', str(s or '')))


NON_ALNUM_RE = re.compile(r"[^a-z0-9]+")


def canonical_text(s: str) -> str:
    lowered = strip_cid_tokens(s).lower()
    lowered = NON_ALNUM_RE.sub(' ', lowered)
    return normalize_ws(lowered)


def to_text_body(card):
    body = card.get('body', '')
    if isinstance(body, list):
        body = ' '.join(str(x) for x in body)
    return normalize_ws(body)


def fingerprint(card):
    filename = canonical_text(card.get('filename', ''))
    tag = canonical_text(card.get('tag', ''))
    highlight = canonical_text(card.get('highlighted_text', ''))
    cite = canonical_text(card.get('cite', ''))
    body = canonical_text(to_text_body(card))
    body_prefix = body[:220]
    highlight_prefix = highlight[:220]

    # content signature avoids dependence on generated IDs
    payload = '||'.join([filename, tag, cite, highlight_prefix, body_prefix])
    digest = hashlib.sha256(payload.encode('utf-8')).hexdigest()
    return digest


def card_label(card):
    return {
        'id': card.get('id'),
        'filename': card.get('filename'),
        'tag': card.get('tag'),
        'cite': card.get('cite'),
        'highlighted_text_prefix': normalize_ws(card.get('highlighted_text', ''))[:160],
    }


py_cards = read_jsonl(PY_INDEX)
rs_cards = read_jsonl(RS_INDEX)

py_fp = Counter(fingerprint(c) for c in py_cards)
rs_fp = Counter(fingerprint(c) for c in rs_cards)

missing_fp = py_fp - rs_fp  # present more times in python than rust
extra_fp = rs_fp - py_fp

# map fp -> example cards
py_fp_examples = defaultdict(list)
for c in py_cards:
    fp = fingerprint(c)
    if len(py_fp_examples[fp]) < 3:
        py_fp_examples[fp].append(card_label(c))

rs_fp_examples = defaultdict(list)
for c in rs_cards:
    fp = fingerprint(c)
    if len(rs_fp_examples[fp]) < 3:
        rs_fp_examples[fp].append(card_label(c))

missing_by_file = Counter()
for fp, n in missing_fp.items():
    examples = py_fp_examples.get(fp, [])
    if examples:
        fn = (examples[0].get('filename') or '').strip()
    else:
        fn = ''
    missing_by_file[fn] += n

extra_by_file = Counter()
for fp, n in extra_fp.items():
    examples = rs_fp_examples.get(fp, [])
    if examples:
        fn = (examples[0].get('filename') or '').strip()
    else:
        fn = ''
    extra_by_file[fn] += n

# Compare ID overlap directly to detect id-generation differences
py_ids = set(str(c.get('id')) for c in py_cards if c.get('id') is not None)
rs_ids = set(str(c.get('id')) for c in rs_cards if c.get('id') is not None)

report = {
    'python_count': len(py_cards),
    'rust_count': len(rs_cards),
    'count_delta_rust_minus_python': len(rs_cards) - len(py_cards),
    'content_missing_in_rust_total': sum(missing_fp.values()),
    'content_extra_in_rust_total': sum(extra_fp.values()),
    'id_overlap': {
        'python_unique_ids': len(py_ids),
        'rust_unique_ids': len(rs_ids),
        'intersection': len(py_ids & rs_ids),
        'only_python': len(py_ids - rs_ids),
        'only_rust': len(rs_ids - py_ids),
    },
    'missing_by_file_top20': missing_by_file.most_common(20),
    'extra_by_file_top20': extra_by_file.most_common(20),
    'missing_examples': [
        {
            'count': n,
            'python_examples': py_fp_examples.get(fp, []),
        }
        for fp, n in missing_fp.most_common(60)
    ],
    'extra_examples': [
        {
            'count': n,
            'rust_examples': rs_fp_examples.get(fp, []),
        }
        for fp, n in extra_fp.most_common(60)
    ],
}

OUT.write_text(json.dumps(report, indent=2), encoding='utf-8')

print(json.dumps({
    'python_count': report['python_count'],
    'rust_count': report['rust_count'],
    'content_missing_in_rust_total': report['content_missing_in_rust_total'],
    'content_extra_in_rust_total': report['content_extra_in_rust_total'],
    'id_overlap': report['id_overlap'],
    'missing_by_file_top10': report['missing_by_file_top20'][:10],
    'extra_by_file_top10': report['extra_by_file_top20'][:10],
}, indent=2))
