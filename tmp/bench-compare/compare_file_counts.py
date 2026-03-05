import json
from pathlib import Path
from collections import Counter

root = Path('/Users/louis/Logos Rust/tmp/bench-compare')
py_index = root / 'python_local_docs' / 'cards_index.json'
rs_index = root / 'rust_local_docs' / 'cards_index.json'


def file_counts(path: Path):
    counts = Counter()
    with path.open('r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            filename = str(obj.get('filename', '')).strip()
            counts[filename] += 1
    return counts


py = file_counts(py_index)
rs = file_counts(rs_index)
all_files = sorted(set(py) | set(rs))

diffs = []
for filename in all_files:
    py_count = py.get(filename, 0)
    rs_count = rs.get(filename, 0)
    if py_count != rs_count:
        diffs.append((rs_count - py_count, filename, py_count, rs_count))

diffs.sort()

print(f'python_total={sum(py.values())}')
print(f'rust_total={sum(rs.values())}')
print(f'delta={sum(rs.values()) - sum(py.values())}')
print('')
print('rust_fewer_top12:')
for delta, filename, py_count, rs_count in diffs[:12]:
    print(f'{delta:>5} | py={py_count:>4} rs={rs_count:>4} | {filename}')
print('')
print('rust_more_top12:')
for delta, filename, py_count, rs_count in diffs[-12:]:
    print(f'{delta:>5} | py={py_count:>4} rs={rs_count:>4} | {filename}')
