import sys, os
from urllib import parse
# Just parse 1nc.docx and use python Search directly!
sys.path.append(os.path.join(os.getcwd(), "verbatim-parser "))
from search import Search
s = Search("tmp/bench-compare/parity_python_docs/cards_index.json")
res = s.query("conservation", match_mode="paragraph", limit=100)
for c in res[0]:
    if c.get("card_identifier") == "CID-00000-00053":
        print("MATCHED 53!")
        print("TAG:", c.get("tag"))
        print("CITE:", c.get("cite"))
        break
