import json
from pathlib import Path
from collections import Counter, defaultdict

root = Path('/Users/louis/Logos Rust/tmp/bench-compare')
pyi = root / 'python_local_docs' / 'cards_index.json'
rsi = root / 'rust_local_docs' / 'cards_index.json'


def load(path: Path):
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


py = load(pyi)
rs = load(rsi)

pyc = Counter(str(c.get('filename', '')).strip() for c in py)
rsc = Counter(str(c.get('filename', '')).strip() for c in rs)

print(f'python_total={len(py)}')
print(f'rust_total={len(rs)}')
print(f'delta={len(rs)-len(py)}')
print('')

print('file_deltas_rust_minus_python:')
for fn, d in sorted(((fn, rsc.get(fn, 0) - pyc.get(fn, 0)) for fn in set(pyc) | set(rsc)), key=lambda x: x[1], reverse=True):
    if d != 0:
        print(f'{d:>4} | py={pyc.get(fn,0):>4} rs={rsc.get(fn,0):>4} | {fn}')

extra_files = [fn for fn in set(rsc) if rsc[fn] - pyc.get(fn, 0) > 0]
issues = defaultdict(Counter)
samples = defaultdict(list)

for card in rs:
    fn = str(card.get('filename', '')).strip()
    if fn not in extra_files:
        continue

    tag = str(card.get('tag', '')).strip()
    cite = str(card.get('cite', '')).strip()
    body = card.get('body', [])
    if isinstance(body, list):
        body_text = ' '.join(str(x) for x in body).strip()
        body_parts = len(body)
    else:
        body_text = str(body).strip()
        body_parts = -1

    if tag == '':
        issues[fn]['empty_tag'] += 1
    if cite == '':
        issues[fn]['empty_cite'] += 1
    if len(body_text) < 25:
        issues[fn]['short_body_lt25'] += 1
    if isinstance(body, list) and len(body) == 0:
        issues[fn]['empty_body_list'] += 1

    if len(samples[fn]) < 3:
        samples[fn].append({
            'tag': tag[:120],
            'cite': cite[:140],
            'body_prefix': body_text[:160],
            'body_len': len(body_text),
            'body_parts': body_parts,
        })

print('')
print('extra_file_validity_summary:')
for fn in sorted(extra_files, key=lambda name: rsc[name] - pyc.get(name, 0), reverse=True):
    d = rsc[fn] - pyc.get(fn, 0)
    print(f'')
    print(f'{fn} | delta={d} | rs={rsc[fn]} py={pyc.get(fn,0)}')
    if issues[fn]:
        print(f'issues={dict(issues[fn])}')
    else:
        print('issues=none')
    for sample in samples[fn]:
        print(f'sample={sample}')
