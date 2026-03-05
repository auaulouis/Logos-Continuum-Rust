import json
import os
import shutil
import socket
import subprocess
import time
import urllib.request
from pathlib import Path

ROOT = Path('/Users/louis/Logos Rust')
BENCH = ROOT / 'tmp' / 'bench-compare'
SOURCE = BENCH / 'source_docs'
RUST_LOCAL = BENCH / 'rust_local_docs'
RUST_INDEX = RUST_LOCAL / 'cards_index.json'


def opener_no_proxy():
    return urllib.request.build_opener(urllib.request.ProxyHandler({}))


HTTP = opener_no_proxy()


def http_json(method, url, payload=None, timeout=60):
    headers = {}
    data = None
    if payload is not None:
        data = json.dumps(payload).encode('utf-8')
        headers['Content-Type'] = 'application/json'
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with HTTP.open(req, timeout=timeout) as resp:
        raw = resp.read().decode('utf-8')
        return json.loads(raw) if raw else {}


def wait_ready(base_url, timeout_s=90):
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            http_json('GET', f'{base_url}/parser-settings', timeout=4)
            return True
        except Exception:
            time.sleep(0.3)
    return False


def pick_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def count_jsonl(path: Path):
    if not path.exists():
        return 0
    n = 0
    with path.open('r', encoding='utf-8') as f:
        for line in f:
            if line.strip():
                n += 1
    return n


def main():
    port = pick_port()
    env = os.environ.copy()
    env.update({
        'PORT': str(port),
        'LOCAL_DOCS_FOLDER': str(RUST_LOCAL),
        'LOCAL_INDEX_PATH': str(RUST_INDEX),
        'PARSER_SETTINGS_PATH': str(RUST_LOCAL / 'parser_settings.json'),
        'PARSER_EVENTS_PATH': str(RUST_LOCAL / 'parser_events.jsonl'),
    })

    cmd = [str(ROOT / 'rust-backend' / 'target' / 'debug' / 'logos-backend')]
    proc = subprocess.Popen(cmd, cwd=str(ROOT / 'rust-backend'), env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    base = f'http://127.0.0.1:{port}'
    try:
        if not wait_ready(base):
            raise RuntimeError('rust backend not ready')

        http_json('POST', f'{base}/clear-index', payload={})

        upload_dir = RUST_LOCAL / 'uploaded_docs'
        upload_dir.mkdir(parents=True, exist_ok=True)
        for existing in upload_dir.glob('*.docx'):
            existing.unlink()

        for src in sorted(SOURCE.glob('*.docx')):
            shutil.copy2(src, upload_dir / src.name)

        docs = sorted([p.name for p in upload_dir.glob('*.docx')])
        for filename in docs:
            http_json('POST', f'{base}/index-document', payload={'filename': filename}, timeout=180)

        print(json.dumps({'docs_indexed': len(docs), 'rust_card_count': count_jsonl(RUST_INDEX)}, indent=2))
    finally:
        try:
            proc.terminate()
            proc.wait(timeout=6)
        except Exception:
            try:
                proc.kill()
            except Exception:
                pass


if __name__ == '__main__':
    main()
