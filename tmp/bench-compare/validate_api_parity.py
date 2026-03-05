import json
import os
import shutil
import socket
import subprocess
import time
import urllib.parse
import urllib.request
import urllib.error
from pathlib import Path

ROOT = Path('/Users/louis/Logos Rust')
BENCH = ROOT / 'tmp' / 'bench-compare'
SOURCE_DOCS = BENCH / 'source_docs'
REPORT_PATH = BENCH / 'api_parity_report.json'
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))
PYTHON_BIN = ROOT / '.venv' / 'bin' / 'python'


def pick_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def http_json(method: str, url: str, payload=None, timeout=60):
    data = None
    headers = {}
    if payload is not None:
        data = json.dumps(payload).encode('utf-8')
        headers['Content-Type'] = 'application/json'
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with HTTP.open(req, timeout=timeout) as resp:
            raw = resp.read().decode('utf-8')
            return resp.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as error:
        raw = error.read().decode('utf-8')
        print("HTTPError:", error.code, raw)
        try:
            payload = json.loads(raw) if raw else {}
        except Exception:
            payload = {'error': raw}
        return error.code, payload
    except Exception as error:
        return 599, {'error': str(error)}


def http_multipart_upload(url: str, filename: str, file_bytes: bytes, parse: bool, timeout=60):
    boundary = f'----logosBoundary{int(time.time() * 1000)}'
    chunks = []

    def add(text: str):
        chunks.append(text.encode('utf-8'))

    add(f'--{boundary}\r\n')
    add(f'Content-Disposition: form-data; name="file"; filename="{filename}"\r\n')
    add('Content-Type: application/vnd.openxmlformats-officedocument.wordprocessingml.document\r\n\r\n')
    chunks.append(file_bytes)
    add('\r\n')

    add(f'--{boundary}\r\n')
    add('Content-Disposition: form-data; name="parse"\r\n\r\n')
    add('true' if parse else 'false')
    add('\r\n')

    add(f'--{boundary}--\r\n')
    body = b''.join(chunks)

    req = urllib.request.Request(
        url,
        data=body,
        method='POST',
        headers={
            'Content-Type': f'multipart/form-data; boundary={boundary}',
            'Content-Length': str(len(body)),
        },
    )
    try:
        with HTTP.open(req, timeout=timeout) as resp:
            raw = resp.read().decode('utf-8')
            return resp.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as error:
        raw = error.read().decode('utf-8')
        print("HTTPError:", error.code, raw)
        try:
            payload = json.loads(raw) if raw else {}
        except Exception:
            payload = {'error': raw}
        return error.code, payload
    except Exception as error:
        return 599, {'error': str(error)}


def wait_ready(base_url: str, timeout_s=120) -> bool:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            status, _ = http_json('GET', f'{base_url}/parser-settings', timeout=4)
            if status == 200:
                return True
        except Exception:
            pass
        time.sleep(0.4)
    return False


def count_cards(index_path: Path) -> int:
    if not index_path.exists():
        return 0
    total = 0
    with index_path.open('r', encoding='utf-8') as handle:
        for line in handle:
            if line.strip():
                total += 1
    return total


def extract_query_terms(index_path: Path, limit=120):
    terms = []
    seen = set()
    with index_path.open('r', encoding='utf-8') as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                card = json.loads(line)
            except Exception:
                continue
            text = ' '.join([
                str(card.get('tag', '')),
                str(card.get('cite', '')),
                str(card.get('highlighted_text', '')),
            ])
            for token in text.split():
                token = ''.join(ch for ch in token.lower() if ch.isalnum() or ch in ('-', '_'))
                if len(token) < 4 or token in seen:
                    continue
                seen.add(token)
                terms.append(token)
                if len(terms) >= limit:
                    return terms
    return terms


def run_backend(name: str, cmd, cwd: Path, env: dict, port: int):
    proc = subprocess.Popen(
        cmd,
        cwd=str(cwd),
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    base = f'http://127.0.0.1:{port}'
    if not wait_ready(base):
        log = ''
        if proc.stdout is not None:
            try:
                log = proc.stdout.read()[-2000:]
            except Exception:
                pass
        proc.kill()
        raise RuntimeError(f'{name} did not become ready: {log}')
    return proc, base


def stop_backend(proc):
    try:
        proc.terminate()
        proc.wait(timeout=8)
    except Exception:
        try:
            proc.kill()
        except Exception:
            pass


def check_eq(failures, a, b, message):
    if a != b:
        failures.append(f'{message}: left={a!r} right={b!r}')


def check_true(failures, condition, message):
    if not condition:
        failures.append(message)


def main():
    docs = sorted(SOURCE_DOCS.glob('*.docx'))
    if not docs:
        raise RuntimeError('No source docs found in tmp/bench-compare/source_docs')

    docs_limit = os.environ.get('PARITY_DOC_LIMIT', '').strip()
    if docs_limit:
        docs = docs[: max(1, int(docs_limit))]
    
    docs = [d for d in docs if "1nc" in d.name]

    py_port = pick_port()
    rs_port = pick_port()

    py_local = BENCH / 'parity_python_docs'
    rs_local = BENCH / 'parity_rust_docs'
    for folder in [py_local, rs_local]:
        shutil.rmtree(folder, ignore_errors=True)
        (folder / 'uploaded_docs').mkdir(parents=True, exist_ok=True)

    py_env = os.environ.copy()
    py_env.update(
        {
            'PORT': str(py_port),
            'LOCAL_DOCS_FOLDER': str(py_local),
            'LOCAL_INDEX_PATH': str(py_local / 'cards_index.json'),
            'PARSER_SETTINGS_PATH': str(py_local / 'parser_settings.json'),
            'PARSER_EVENTS_PATH': str(py_local / 'parser_events.jsonl'),
        }
    )

    rs_env = os.environ.copy()
    rs_env.update(
        {
            'PORT': str(rs_port),
            'LOCAL_DOCS_FOLDER': str(rs_local),
            'LOCAL_INDEX_PATH': str(rs_local / 'cards_index.json'),
            'INDEX_CACHE_PATH': str(rs_local / 'cards_cache.bin'),
            'PARSER_SETTINGS_PATH': str(rs_local / 'parser_settings.json'),
            'PARSER_EVENTS_PATH': str(rs_local / 'parser_events.jsonl'),
        }
    )

    rust_binary = ROOT / 'rust-backend' / 'target' / 'debug' / 'logos-backend'
    if not rust_binary.exists():
        raise RuntimeError('Build rust-backend first: missing target/debug/logos-backend')

    py_proc, py_base = run_backend(
        'python',
        [
            str(PYTHON_BIN),
            '-c',
            f'import api; api.app.run(port={py_port}, host="0.0.0.0", debug=False, use_reloader=False)',
        ],
        ROOT / 'verbatim-parser ',
        py_env,
        py_port,
    )

    rs_proc, rs_base = run_backend('rust', [str(rust_binary)], ROOT / 'rust-backend', rs_env, rs_port)

    failures = []
    query_failures = []

    try:
        for base in [py_base, rs_base]:
            status, payload = http_json('POST', f'{base}/clear-index', payload={})
            check_true(failures, status == 200 and payload.get('ok') is True, f'clear-index failed on {base}: {payload}')

        # upload/index flow parity across whole corpus
        uploaded_pairs = []
        for doc in docs:
            doc_bytes = doc.read_bytes()
            py_upload_status, py_upload = http_multipart_upload(
                f'{py_base}/upload-docx', doc.name, doc_bytes, parse=False, timeout=90
            )
            rs_upload_status, rs_upload = http_multipart_upload(
                f'{rs_base}/upload-docx', doc.name, doc_bytes, parse=False, timeout=90
            )
            check_eq(failures, py_upload_status, 200, f'python upload status: {doc.name}')
            check_eq(failures, rs_upload_status, 200, f'rust upload status: {doc.name}')
            check_eq(failures, py_upload.get('ok'), True, f'python upload ok: {doc.name}')
            check_eq(failures, rs_upload.get('ok'), True, f'rust upload ok: {doc.name}')
            py_name = py_upload.get('filename')
            rs_name = rs_upload.get('filename')
            check_true(failures, bool(py_name and rs_name), f'upload missing filename: {doc.name}')
            if py_name and rs_name:
                uploaded_pairs.append((doc.name, py_name, rs_name))

        for source_name, py_name, rs_name in uploaded_pairs:
            py_index_status, py_index_payload = http_json(
                'POST', f'{py_base}/index-document', {'filename': py_name}, timeout=240
            )
            rs_index_status, rs_index_payload = http_json(
                'POST', f'{rs_base}/index-document', {'filename': rs_name}, timeout=240
            )
            check_eq(failures, py_index_status, 200, f'python index status: {source_name}')
            check_eq(failures, rs_index_status, 200, f'rust index status: {source_name}')
            check_true(
                failures,
                py_index_payload.get('cards_indexed', 0) > 0 and rs_index_payload.get('cards_indexed', 0) > 0,
                f'index-document produced no cards: {source_name}',
            )

        py_count = count_cards(py_local / 'cards_index.json')
        rs_count = count_cards(rs_local / 'cards_index.json')
        check_eq(failures, py_count, rs_count, 'card count parity')

        # parser-settings behavior parity
        settings_payload = {
            'use_parallel_processing': 'false',
            'parser_card_workers': 9999,
            'local_parser_file_workers': 0,
            'flush_enabled': '1',
            'flush_every_docs': 0,
        }
        _, py_settings = http_json('POST', f'{py_base}/parser-settings', settings_payload)
        _, rs_settings = http_json('POST', f'{rs_base}/parser-settings', settings_payload)
        check_eq(failures, py_settings.get('settings'), rs_settings.get('settings'), 'parser settings normalization parity')

        # query parity on terms sampled from corpus
        terms = extract_query_terms(py_local / 'cards_index.json', int(os.environ.get('PARITY_TERM_LIMIT', '120')))
        query_variants = [
            {'cursor': 0, 'limit': 30},
            {'cursor': 0, 'limit': 15, 'sort_by': 'date'},
            {'cursor': 5, 'limit': 20},
            {'cursor': 0, 'limit': 25, 'match_mode': 'paragraph'},
            {'cursor': 0, 'limit': 25, 'match_mode': 'tag'},
        ]

        for term in terms:
            for variant in query_variants:
                payload = {'search': term}
                payload.update(variant)
                query = urllib.parse.urlencode(payload)
                _, py_q = http_json('GET', f'{py_base}/query?{query}', timeout=60)
                _, rs_q = http_json('GET', f'{rs_base}/query?{query}', timeout=60)
                py_ids = [item.get('card_identifier') for item in py_q.get('results', [])]
                rs_ids = [item.get('card_identifier') for item in rs_q.get('results', [])]
                if py_q.get('count') != rs_q.get('count') or py_ids != rs_ids:
                    query_failures.append(
                        {
                            'term': term,
                            'variant': variant,
                            'python_count': py_q.get('count'),
                            'rust_count': rs_q.get('count'),
                            'python_ids': py_ids,
                            'rust_ids': rs_ids,
                            'diff_py_minus_rs': list(set(py_ids) - set(rs_ids)),
                            'diff_rs_minus_py': list(set(rs_ids) - set(py_ids)),
                        }
                    )

        if query_failures:
            failures.append(f'query parity mismatches: {len(query_failures)}')

        # parser-events shape and error semantics parity
        _, py_events = http_json('GET', f'{py_base}/parser-events?limit=25')
        _, rs_events = http_json('GET', f'{rs_base}/parser-events?limit=25')
        py_list = py_events.get('events', [])
        rs_list = rs_events.get('events', [])
        check_true(failures, isinstance(py_list, list) and isinstance(rs_list, list), 'parser-events payload must be list')
        for item in py_list[:5] + rs_list[:5]:
            check_true(failures, isinstance(item, dict), 'parser-events entries must be object')
            check_true(failures, 'level' in item and 'message' in item, 'parser-events entries missing level/message')

        # documents parity after indexing
        _, py_docs = http_json('GET', f'{py_base}/documents')
        _, rs_docs = http_json('GET', f'{rs_base}/documents')
        py_doc_map = {str(d.get('filename', '')).lower(): d for d in py_docs.get('documents', [])}
        rs_doc_map = {str(d.get('filename', '')).lower(): d for d in rs_docs.get('documents', [])}
        check_eq(failures, sorted(py_doc_map.keys()), sorted(rs_doc_map.keys()), 'documents filename set parity')


        report = {
            'ok': len(failures) == 0,
            'docs_checked': len(docs),
            'card_count_python': py_count,
            'card_count_rust': rs_count,
            'query_terms_checked': len(terms),
            'query_variants_checked': len(query_variants),
            'query_mismatch_count': len(query_failures),
            'query_mismatch_examples': query_failures[:25],
            'python_port': py_port,
            'rust_port': rs_port,
            'failures': failures,
        }
        REPORT_PATH.write_text(json.dumps(report, indent=2), encoding='utf-8')
        print(json.dumps(report, indent=2))
        if failures:
            raise SystemExit(1)
    finally:
        stop_backend(py_proc)
        stop_backend(rs_proc)


if __name__ == '__main__':
    main()
