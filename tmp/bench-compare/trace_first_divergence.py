import json
import re
from pathlib import Path
from collections import defaultdict
from difflib import SequenceMatcher

ROOT = Path('/Users/louis/Logos Rust')
BENCH = ROOT / 'tmp' / 'bench-compare'
PY_INDEX = BENCH / 'python_local_docs' / 'cards_index.json'
RS_INDEX = BENCH / 'rust_local_docs' / 'cards_index.json'
OUT = BENCH / 'first_divergence_report.json'

TARGET_FILES = [
    'Mega LD Backfile Part 1 .docx',
    'Mega LD Backfile Part 2 Impacts Defense.docx',
    'Mega LD Backfile Part 2 Imapcts Calculus Malthus Cap copy.docx',
]

CID_RE = re.compile(r"\s*\[\[CID-[^\]]+\]\]\s*", re.IGNORECASE)
NON_ALNUM_RE = re.compile(r"[^a-z0-9]+")


def normalize_ws(s: str) -> str:
    return ' '.join(str(s or '').split())


def canonical_text(s: str) -> str:
    lowered = normalize_ws(CID_RE.sub(' ', str(s or ''))).lower()
    lowered = NON_ALNUM_RE.sub(' ', lowered)
    return normalize_ws(lowered)


def as_body_text(card):
    body = card.get('body', '')
    if isinstance(body, list):
        body = ' '.join(str(x) for x in body)
    return str(body or '')


def card_key(card):
    filename = canonical_text(card.get('filename', ''))
    tag = canonical_text(card.get('tag', ''))
    cite = canonical_text(card.get('cite', ''))
    highlight = canonical_text(card.get('highlighted_text', ''))[:180]
    body = canonical_text(as_body_text(card))[:180]
    return '||'.join([filename, tag, cite, highlight, body])


def card_preview(card):
    return {
        'id': card.get('id'),
        'tag': normalize_ws(card.get('tag', ''))[:220],
        'cite': normalize_ws(card.get('cite', ''))[:220],
        'highlight': normalize_ws(card.get('highlighted_text', ''))[:220],
        'body': normalize_ws(as_body_text(card))[:220],
    }


def load_by_file(path: Path):
    out = defaultdict(list)
    with path.open('r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                card = json.loads(line)
            except json.JSONDecodeError:
                continue
            if not isinstance(card, dict):
                continue
            filename = str(card.get('filename', '')).strip()
            out[filename].append(card)
    return out


def first_divergence(py_cards, rs_cards):
    py_keys = [card_key(c) for c in py_cards]
    rs_keys = [card_key(c) for c in rs_cards]

    matcher = SequenceMatcher(a=py_keys, b=rs_keys, autojunk=False)
    blocks = matcher.get_matching_blocks()

    first_nonzero = None
    for block in blocks:
        if block.size > 0:
            continue
        # size=0 sentinel at end; skip
    opcodes = matcher.get_opcodes()
    for tag, i1, i2, j1, j2 in opcodes:
        if tag != 'equal':
            first_nonzero = (tag, i1, i2, j1, j2)
            break

    if first_nonzero is None:
        return {
            'equal': True,
            'python_count': len(py_cards),
            'rust_count': len(rs_cards),
        }

    tag, i1, i2, j1, j2 = first_nonzero
    py_start = max(0, i1 - 2)
    py_end = min(len(py_cards), i2 + 3)
    rs_start = max(0, j1 - 2)
    rs_end = min(len(rs_cards), j2 + 3)

    return {
        'equal': False,
        'python_count': len(py_cards),
        'rust_count': len(rs_cards),
        'first_opcode': {
            'tag': tag,
            'python_range': [i1, i2],
            'rust_range': [j1, j2],
        },
        'python_window': [
            {
                'index': idx,
                'card': card_preview(py_cards[idx]),
            }
            for idx in range(py_start, py_end)
        ],
        'rust_window': [
            {
                'index': idx,
                'card': card_preview(rs_cards[idx]),
            }
            for idx in range(rs_start, rs_end)
        ],
    }


def main():
    py_by_file = load_by_file(PY_INDEX)
    rs_by_file = load_by_file(RS_INDEX)

    report = {
        'targets': TARGET_FILES,
        'docs': {},
    }

    for name in TARGET_FILES:
        py_cards = py_by_file.get(name, [])
        rs_cards = rs_by_file.get(name, [])
        report['docs'][name] = first_divergence(py_cards, rs_cards)

    OUT.write_text(json.dumps(report, indent=2), encoding='utf-8')

    summary = {}
    for name in TARGET_FILES:
        item = report['docs'][name]
        if item.get('equal'):
            summary[name] = {
                'equal': True,
                'python_count': item['python_count'],
                'rust_count': item['rust_count'],
            }
        else:
            summary[name] = {
                'equal': False,
                'python_count': item['python_count'],
                'rust_count': item['rust_count'],
                'first_opcode': item['first_opcode'],
                'python_first_tag': item['python_window'][0]['card']['tag'] if item['python_window'] else '',
                'rust_first_tag': item['rust_window'][0]['card']['tag'] if item['rust_window'] else '',
            }

    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
