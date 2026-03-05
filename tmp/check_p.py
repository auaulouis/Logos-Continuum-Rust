import json
d = json.load(open("tmp/bench-compare/api_parity_report.json"))
for q in d.get("query_parity_mismatches", []):
    if q.get("variant", {}).get("match_mode") == "paragraph":
        p = q.get("diff_py_minus_rs", [])
        r = q.get("diff_rs_minus_py", [])
        if p or r:
            print("Term:", q["term"])
            if p: print("  Py :", p)
            if r: print("  Rs :", r)
