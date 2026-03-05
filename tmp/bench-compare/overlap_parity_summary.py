import json
from pathlib import Path
from collections import Counter

root = Path('/Users/louis/Logos Rust/tmp/bench-compare')
pyi = root / 'python_local_docs' / 'cards_index.json'
rsi = root / 'rust_local_docs' / 'cards_index.json'

def load(path):
    rows=[]
    with open(path,'r',encoding='utf-8') as f:
        for line in f:
            line=line.strip()
            if not line:
                continue
            try:
                o=json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(o,dict):
                rows.append(o)
    return rows

py=load(pyi); rs=load(rsi)
pyc=Counter(str(c.get('filename','')).strip() for c in py)
rsc=Counter(str(c.get('filename','')).strip() for c in rs)

overlap_docs=[fn for fn in pyc if pyc[fn] > 0]
overlap_py=sum(pyc[fn] for fn in overlap_docs)
overlap_rs=sum(rsc.get(fn,0) for fn in overlap_docs)

abs_delta=sum(abs(rsc.get(fn,0)-pyc[fn]) for fn in overlap_docs)
exact_match=sum(1 for fn in overlap_docs if rsc.get(fn,0)==pyc[fn])
within_1=sum(1 for fn in overlap_docs if abs(rsc.get(fn,0)-pyc[fn])<=1)
within_5=sum(1 for fn in overlap_docs if abs(rsc.get(fn,0)-pyc[fn])<=5)

largest=sorted(((rsc.get(fn,0)-pyc[fn], fn, pyc[fn], rsc.get(fn,0)) for fn in overlap_docs), key=lambda x: abs(x[0]), reverse=True)[:10]

print('overlap_docs', len(overlap_docs))
print('overlap_py_total', overlap_py)
print('overlap_rs_total', overlap_rs)
print('overlap_delta_rs_minus_py', overlap_rs-overlap_py)
print('sum_abs_doc_deltas', abs_delta)
print('exact_match_docs', exact_match)
print('within_1_docs', within_1)
print('within_5_docs', within_5)
print('largest_doc_deltas:')
for d,fn,pv,rv in largest:
    print(f'{d:>4} | py={pv:>4} rs={rv:>4} | {fn}')
