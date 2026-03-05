#!/usr/bin/env python3
import argparse
import json
import os
import random
import shutil
import signal
import subprocess
import tempfile
import time
import urllib.request
from pathlib import Path


def dir_size_bytes(path: Path) -> int:
    total = 0
    if not path.exists():
        return 0
    for p in path.rglob('*'):
        if p.is_file():
            total += p.stat().st_size
    return total


def file_count(path: Path) -> int:
    if not path.exists():
        return 0
    return sum(1 for p in path.rglob('*') if p.is_file())


def poll_backend(url: str, timeout_s: float = 20.0) -> bool:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=1.5) as response:
                if 200 <= response.status < 500:
                    return True
        except Exception:
            time.sleep(0.2)
    return False


def read_rss_kb(pid: int) -> int | None:
    try:
        output = subprocess.check_output(['ps', '-o', 'rss=', '-p', str(pid)], text=True).strip()
        return int(output) if output else None
    except Exception:
        return None


def backend_metrics(workspace_root: Path) -> dict:
    backend_bin = workspace_root / 'rust-backend' / 'target' / 'release' / 'logos-backend'
    if not backend_bin.exists():
        return {
            'backend_binary_present': False,
            'backend_startup_ms': None,
            'backend_rss_kb': None,
            'backend_crashed_before_ready': None,
        }

    port = random.randint(5610, 5699)
    temp_docs = Path(tempfile.mkdtemp(prefix='logos-metrics-local-docs-'))
    env = os.environ.copy()
    env.update({
        'PORT': str(port),
        'LOCAL_DOCS_FOLDER': str(temp_docs),
        'LOCAL_INDEX_PATH': str(temp_docs / 'cards_index.json'),
        'INDEX_CACHE_PATH': str(temp_docs / 'cards_cache.bin'),
        'PARSER_SETTINGS_PATH': str(temp_docs / 'parser_settings.json'),
        'PARSER_EVENTS_PATH': str(temp_docs / 'parser_events.jsonl'),
    })

    process = None
    started = time.perf_counter()
    try:
        process = subprocess.Popen(
            [str(backend_bin)],
            cwd=str(backend_bin.parent),
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            preexec_fn=os.setsid,
        )

        ready = poll_backend(f'http://127.0.0.1:{port}/health')
        startup_ms = round((time.perf_counter() - started) * 1000.0, 2)
        rss_kb = read_rss_kb(process.pid)
        crashed_before_ready = process.poll() is not None and not ready

        return {
            'backend_binary_present': True,
            'backend_startup_ms': startup_ms,
            'backend_rss_kb': rss_kb,
            'backend_crashed_before_ready': crashed_before_ready,
            'backend_ready': ready,
        }
    finally:
        if process and process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except Exception:
                process.terminate()
        shutil.rmtree(temp_docs, ignore_errors=True)


def main() -> None:
    parser = argparse.ArgumentParser(description='Collect desktop build and runtime metrics.')
    parser.add_argument('--mode', required=True, choices=['electron', 'tauri'])
    parser.add_argument('--workspace-root', required=True)
    parser.add_argument('--artifact-dir', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()

    workspace_root = Path(args.workspace_root)
    artifact_dir = Path(args.artifact_dir)

    data = {
        'mode': args.mode,
        'collected_at_epoch_ms': int(time.time() * 1000),
        'artifact_dir': str(artifact_dir),
        'artifact_exists': artifact_dir.exists(),
        'artifact_file_count': file_count(artifact_dir),
        'artifact_total_size_bytes': dir_size_bytes(artifact_dir),
        'desktop_startup_ms': None,
        'desktop_rss_kb': None,
        'desktop_crash_rate_window': None,
    }
    data.update(backend_metrics(workspace_root))

    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(data, indent=2))


if __name__ == '__main__':
    main()
