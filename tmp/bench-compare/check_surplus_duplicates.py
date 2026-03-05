import json
import hashlib
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
            if not line: continue
            try: o=json.loads(line)
            except: continue
            if isinstance(o,dict): rows.append(o)
    return rows

py=load(pyi); rs=load(rsi)
pyc=Counter(str(c.get('filename','')).strip() for c in py)
rsc=Counter(str(c.get('filename','')).strip() for c in rs)
extra_files=[fn for fn in rsc if rsc[fn]-pyc.get(fn,0)>0]

for fn in sorted(extra_files, key=lambda f:rsc[f]-pyc.get(f,0), reverse=True):
    cards=[c for c in rs if str(c.get('filename','')).strip()==fn]
    sigs=[]
    for c in cards:
        body=c.get('body',[])
        if isinstance(body,list):
            body='\n'.join(str(x) for x in body)
        else:
            body=str(body)
        src='\n'.join([
            str(c.get('tag_base','')),
            str(c.get('cite','')),
            body,
        ])
        sigs.append(hashlib.sha256(src.encode('utf-8')).hexdigest())
    dup=sum(v-1 for v in Counter(sigs).values() if v>1)
    print(f'{fn} | cards={len(cards)} dup_by_content={dup}')
