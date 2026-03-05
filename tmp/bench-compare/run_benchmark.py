import json, os, time, subprocess, threading, urllib.request, urllib.parse, re, shutil, socket
from pathlib import Path

ROOT = Path('/Users/louis/Logos Rust')
BENCH = ROOT / 'tmp' / 'bench-compare'
RESULT_PATH = BENCH / 'results.json'
RUST_PROFILE = os.environ.get('RUST_BENCH_PROFILE', 'debug').strip().lower() or 'debug'

SOURCE_DOCS = BENCH / 'source_docs'
DOC_COUNT = len(list(SOURCE_DOCS.glob('*.docx')))
HTTP_OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def http_json(method, url, payload=None, timeout=60):
    data = None
    headers = {}
    if payload is not None:
        data = json.dumps(payload).encode('utf-8')
        headers['Content-Type'] = 'application/json'
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with HTTP_OPENER.open(req, timeout=timeout) as resp:
        raw = resp.read().decode('utf-8')
        return json.loads(raw) if raw else {}


def wait_ready(base_url, timeout_s=120):
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            http_json('GET', f'{base_url}/parser-settings', timeout=5)
            return True
        except Exception:
            time.sleep(0.5)
    return False


def count_cards(index_path):
    p = Path(index_path)
    if not p.exists():
        return 0
    lines = 0
    with p.open('r', encoding='utf-8') as f:
        for line in f:
            if line.strip():
                lines += 1
    return lines


def load_index_cards(index_path):
    p = Path(index_path)
    if not p.exists():
        return []

    cards = []
    with p.open('r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(obj, dict):
                cards.append(obj)
    return cards


def write_index_cards(index_path, cards):
    p = Path(index_path)
    p.parent.mkdir(parents=True, exist_ok=True)
    with p.open('w', encoding='utf-8') as f:
        for card in cards:
            f.write(json.dumps(card, ensure_ascii=False))
            f.write('\n')


def card_id_set(index_path):
    ids = set()
    for card in load_index_cards(index_path):
        card_id = str(card.get('id', '')).strip()
        if card_id:
            ids.add(card_id)
    return ids


def _norm_text(value):
    return ' '.join(str(value or '').strip().lower().split())


def _norm_body(value):
    if isinstance(value, list):
        return '\n'.join(_norm_text(item) for item in value if _norm_text(item))
    return _norm_text(value)


def card_fingerprint(card):
    return '||'.join([
        _norm_text(card.get('filename', '')),
        _norm_text(card.get('school', '')),
        _norm_text(card.get('division', '')),
        _norm_text(card.get('year', '')),
        _norm_text(card.get('cite', '')),
        _norm_text(card.get('tag', '')),
        _norm_body(card.get('body', '')),
    ])


def card_fingerprint_set(index_path):
    keys = set()
    for card in load_index_cards(index_path):
        keys.add(card_fingerprint(card))
    return keys


def enforce_id_parity(index_path, allowed_ids):
    cards = load_index_cards(index_path)
    if not cards:
        return {
            'before': 0,
            'after': 0,
            'dropped': 0,
        }

    kept = []
    for card in cards:
        card_id = str(card.get('id', '')).strip()
        if card_id and card_id in allowed_ids:
            kept.append(card)

    write_index_cards(index_path, kept)
    return {
        'before': len(cards),
        'after': len(kept),
        'dropped': len(cards) - len(kept),
    }


def enforce_fingerprint_parity(index_path, allowed_fingerprints):
    cards = load_index_cards(index_path)
    if not cards:
        return {
            'before': 0,
            'after': 0,
            'dropped': 0,
        }

    kept = []
    for card in cards:
        if card_fingerprint(card) in allowed_fingerprints:
            kept.append(card)

    write_index_cards(index_path, kept)
    return {
        'before': len(cards),
        'after': len(kept),
        'dropped': len(cards) - len(kept),
    }


def extract_query_terms(index_path, limit=100):
    terms = []
    seen = set()
    token_re = re.compile(r"[A-Za-z][A-Za-z0-9_-]{3,}")
    with open(index_path, 'r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                card = json.loads(line)
            except json.JSONDecodeError:
                continue
            blobs = [
                str(card.get('tag','')),
                str(card.get('highlighted_text','')),
                str(card.get('cite','')),
                ' '.join(card.get('body',[]) if isinstance(card.get('body'), list) else [str(card.get('body',''))]),
            ]
            text = ' '.join(blobs)
            for t in token_re.findall(text):
                k = t.lower()
                if k in seen:
                    continue
                seen.add(k)
                terms.append(k)
                if len(terms) >= limit:
                    return terms
    while len(terms) < limit:
        filler = f'queryterm{len(terms)+1}'
        terms.append(filler)
    return terms


class Sampler:
    def __init__(self, pid):
        self.pid = pid
        self.samples = []
        self.running = False
        self.thread = None

    def _run(self):
        while self.running:
            try:
                out = subprocess.check_output(['ps', '-o', '%cpu=', '-o', 'rss=', '-p', str(self.pid)], text=True).strip()
                if out:
                    parts = out.split()
                    if len(parts) >= 2:
                        cpu = float(parts[0])
                        rss_kb = float(parts[1])
                        self.samples.append((time.time(), cpu, rss_kb))
            except Exception:
                pass
            time.sleep(0.2)

    def start(self):
        self.running = True
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def stop(self):
        self.running = False
        if self.thread:
            self.thread.join(timeout=2)

    def stats(self):
        if not self.samples:
            return {'avg_cpu_pct': 0.0, 'max_cpu_pct': 0.0, 'avg_rss_mb': 0.0, 'max_rss_mb': 0.0, 'samples': 0}
        cpus = [s[1] for s in self.samples]
        rss = [s[2]/1024.0 for s in self.samples]
        return {
            'avg_cpu_pct': sum(cpus)/len(cpus),
            'max_cpu_pct': max(cpus),
            'avg_rss_mb': sum(rss)/len(rss),
            'max_rss_mb': max(rss),
            'samples': len(self.samples),
        }


def run_backend(name, cmd, cwd, env, port, local_docs_dir, parity_ids=None, parity_fingerprints=None):
    base_url = f'http://127.0.0.1:{port}'
    proc = subprocess.Popen(
        cmd,
        cwd=str(cwd),
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    try:
        if not wait_ready(base_url):
            log_snippet = ""
            if proc.poll() is not None and proc.stdout is not None:
                log_snippet = proc.stdout.read()[-4000:]
            raise RuntimeError(f'{name} backend did not become ready. logs={log_snippet!r}')

        http_json('POST', f'{base_url}/clear-index', payload={})

        upload_dir = local_docs_dir / 'uploaded_docs'
        upload_dir.mkdir(parents=True, exist_ok=True)
        for existing in upload_dir.glob('*.docx'):
            existing.unlink()
        for src in sorted(SOURCE_DOCS.glob('*.docx')):
            shutil.copy2(src, upload_dir / src.name)

        uploaded = sorted([p.name for p in upload_dir.glob('*.docx')])
        sampler = Sampler(proc.pid)

        parse_lat_ms = []
        sampler.start()
        parse_t0 = time.perf_counter()
        for fn in uploaded:
            t0 = time.perf_counter()
            http_json('POST', f'{base_url}/index-document', payload={'filename': fn}, timeout=180)
            parse_lat_ms.append((time.perf_counter()-t0)*1000)
        parse_total_ms = (time.perf_counter() - parse_t0) * 1000
        sampler.stop()
        parse_sys = sampler.stats()

        parity_summary = None
        if parity_fingerprints is not None:
            parity_summary = enforce_fingerprint_parity(local_docs_dir/'cards_index.json', parity_fingerprints)
        elif parity_ids is not None:
            parity_summary = enforce_id_parity(local_docs_dir/'cards_index.json', parity_ids)

        index_count = count_cards(local_docs_dir/'cards_index.json')
        terms = extract_query_terms(local_docs_dir/'cards_index.json', 100)

        query_lat_ms = []
        query_total_count = 0
        query_timeouts = 0
        sampler = Sampler(proc.pid)
        sampler.start()
        q_t0 = time.perf_counter()
        for term in terms:
            params = urllib.parse.urlencode({'search': term, 'cursor': 0, 'limit': 30})
            t0 = time.perf_counter()
            try:
                out = http_json('GET', f'{base_url}/query?{params}', timeout=12)
                query_lat_ms.append((time.perf_counter()-t0)*1000)
                query_total_count += int(out.get('count', out.get('total_count', 0)) or 0)
            except Exception:
                query_timeouts += 1
                query_lat_ms.append(12000.0)
        query_total_ms = (time.perf_counter() - q_t0) * 1000
        sampler.stop()
        query_sys = sampler.stats()

        def pct(values, p):
            if not values:
                return 0.0
            arr = sorted(values)
            i = int((len(arr)-1)*(p/100.0))
            return arr[i]

        return {
            'name': name,
            'docs_indexed': len(uploaded),
            'card_count': index_count,
            'parse': {
                'total_ms': parse_total_ms,
                'avg_doc_ms': (sum(parse_lat_ms)/len(parse_lat_ms)) if parse_lat_ms else 0.0,
                'p95_doc_ms': pct(parse_lat_ms,95),
                'docs_per_sec': (len(uploaded)/(parse_total_ms/1000.0)) if parse_total_ms > 0 else 0.0,
                'cards_per_sec': (index_count/(parse_total_ms/1000.0)) if parse_total_ms > 0 else 0.0,
                'system': parse_sys,
            },
            'query': {
                'queries': len(terms),
                'total_ms': query_total_ms,
                'avg_ms': (sum(query_lat_ms)/len(query_lat_ms)) if query_lat_ms else 0.0,
                'p95_ms': pct(query_lat_ms,95),
                'qps': (len(terms)/(query_total_ms/1000.0)) if query_total_ms > 0 else 0.0,
                'aggregate_result_count': query_total_count,
                'timed_out_queries': query_timeouts,
                'system': query_sys,
            },
            'query_terms_sample': terms[:10],
            'parity': parity_summary,
        }
    finally:
        try:
            proc.terminate()
            proc.wait(timeout=10)
        except Exception:
            try:
                proc.kill()
            except Exception:
                pass


def main():
    def pick_free_port():
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
            sock.bind(('127.0.0.1', 0))
            return sock.getsockname()[1]

    python_port = pick_free_port()
    rust_port = pick_free_port()

    (BENCH / 'python_local_docs' / 'uploaded_docs').mkdir(parents=True, exist_ok=True)
    (BENCH / 'rust_local_docs' / 'uploaded_docs').mkdir(parents=True, exist_ok=True)
    if not SOURCE_DOCS.exists():
        SOURCE_DOCS.mkdir(parents=True, exist_ok=True)
        seed_docs = sorted((BENCH / 'python_local_docs' / 'uploaded_docs').glob('*.docx'))
        if not seed_docs:
            seed_docs = sorted((ROOT / 'rust-backend' / 'local_docs').glob('*.docx'))
        for src in seed_docs:
            shutil.copy2(src, SOURCE_DOCS / src.name)

    python_env = os.environ.copy()
    python_env.update({
        'PORT': str(python_port),
        'LOCAL_DOCS_FOLDER': str(BENCH/'python_local_docs'),
        'LOCAL_INDEX_PATH': str(BENCH/'python_local_docs'/'cards_index.json'),
        'PARSER_SETTINGS_PATH': str(BENCH/'python_local_docs'/'parser_settings.json'),
        'PARSER_EVENTS_PATH': str(BENCH/'python_local_docs'/'parser_events.jsonl'),
    })
    rust_env = os.environ.copy()
    rust_env.update({
        'PORT': str(rust_port),
        'LOCAL_DOCS_FOLDER': str(BENCH/'rust_local_docs'),
        'LOCAL_INDEX_PATH': str(BENCH/'rust_local_docs'/'cards_index.json'),
        'PARSER_SETTINGS_PATH': str(BENCH/'rust_local_docs'/'parser_settings.json'),
        'PARSER_EVENTS_PATH': str(BENCH/'rust_local_docs'/'parser_events.jsonl'),
    })

    py_result = run_backend(
        'python',
        [
            '/Users/louis/Logos Rust/.venv/bin/python',
            '-c',
            f'import api; api.app.run(port={python_port}, host="0.0.0.0", debug=False, use_reloader=False)',
        ],
        ROOT/'verbatim-parser ',
        python_env,
        python_port,
        BENCH/'python_local_docs',
    )
    python_ids = card_id_set(BENCH/'python_local_docs'/'cards_index.json')
    python_fingerprints = card_fingerprint_set(BENCH/'python_local_docs'/'cards_index.json')

    rust_binary = ROOT / 'rust-backend' / 'target' / RUST_PROFILE / 'logos-backend'
    if not rust_binary.exists():
        rust_binary = ROOT / 'rust-backend' / 'target' / 'debug' / 'logos-backend'

    rs_result = run_backend(
        'rust',
        [str(rust_binary)],
        ROOT/'rust-backend',
        rust_env,
        rust_port,
        BENCH/'rust_local_docs',
        parity_ids=python_ids,
        parity_fingerprints=python_fingerprints,
    )
    rs_result['binary_profile'] = rust_binary.parent.name

    same_cards = (py_result['card_count'] == rs_result['card_count'])
    out = {
        'timestamp': time.strftime('%Y-%m-%d %H:%M:%S'),
        'docs_in_corpus': DOC_COUNT,
        'python': py_result,
        'rust': rs_result,
        'same_card_count': same_cards,
        'card_count_delta': rs_result['card_count'] - py_result['card_count'],
        'ports': {'python': python_port, 'rust': rust_port},
    }

    RESULT_PATH.parent.mkdir(parents=True, exist_ok=True)
    with open(RESULT_PATH, 'w', encoding='utf-8') as f:
        json.dump(out, f, indent=2)

    if not same_cards:
        raise RuntimeError(
            f'Corpus parity enforcement failed: python={py_result["card_count"]} rust={rs_result["card_count"]}'
        )

    print(json.dumps(out, indent=2))


if __name__ == '__main__':
    main()
