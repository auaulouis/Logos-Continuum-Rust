import re
with open("tmp/bench-compare/validate_api_parity.py", "r") as f: s = f.read()
s = s.replace("except urllib.error.HTTPError as error:\n        raw = error.read().decode(\x27utf-8\x27)", "except urllib.error.HTTPError as error:\n        raw = error.read().decode(\x27utf-8\x27)\n        print(\"HTTPError:\", error.code, raw)")
with open("tmp/bench-compare/validate_api_parity.py", "w") as f: f.write(s)
