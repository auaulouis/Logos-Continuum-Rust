import json
from pathlib import Path

ROOT = Path('/Users/louis/Logos Rust')
BENCH = ROOT / 'tmp' / 'bench-compare'
RESULTS_PATH = BENCH / 'results.json'
THRESHOLDS_PATH = BENCH / 'regression_thresholds.json'


def load_json(path: Path):
    with path.open('r', encoding='utf-8') as handle:
        return json.load(handle)


def main():
    results = load_json(RESULTS_PATH)
    thresholds = load_json(THRESHOLDS_PATH)
    failures = []

    rust = results.get('rust', {})
    rust_parse = rust.get('parse', {})
    rust_query = rust.get('query', {})

    parse_thresholds = thresholds['rust']['parse']
    query_thresholds = thresholds['rust']['query']
    parity_thresholds = thresholds['rust']['parity']

    def check_min(actual, minimum, label):
        if actual < minimum:
            failures.append(f'{label}: actual={actual} < min={minimum}')

    def check_max(actual, maximum, label):
        if actual > maximum:
            failures.append(f'{label}: actual={actual} > max={maximum}')

    check_min(float(rust_parse.get('docs_per_sec', 0.0)), parse_thresholds['docs_per_sec_min'], 'rust.parse.docs_per_sec')
    check_min(float(rust_parse.get('cards_per_sec', 0.0)), parse_thresholds['cards_per_sec_min'], 'rust.parse.cards_per_sec')
    check_max(float(rust_parse.get('p95_doc_ms', 0.0)), parse_thresholds['p95_doc_ms_max'], 'rust.parse.p95_doc_ms')
    check_max(
        float(rust_parse.get('system', {}).get('max_rss_mb', 0.0)),
        parse_thresholds['max_rss_mb_max'],
        'rust.parse.system.max_rss_mb',
    )

    check_max(float(rust_query.get('p95_ms', 0.0)), query_thresholds['p95_ms_max'], 'rust.query.p95_ms')
    check_min(float(rust_query.get('qps', 0.0)), query_thresholds['qps_min'], 'rust.query.qps')
    check_max(int(rust_query.get('timed_out_queries', 0)), query_thresholds['timed_out_queries_max'], 'rust.query.timed_out_queries')
    check_max(
        float(rust_query.get('system', {}).get('max_rss_mb', 0.0)),
        query_thresholds['max_rss_mb_max'],
        'rust.query.system.max_rss_mb',
    )

    delta = int(results.get('card_count_delta', 0))
    allowed = int(parity_thresholds['allow_card_count_delta'])
    if abs(delta) > allowed:
        failures.append(f'card_count_delta: actual={delta} exceeds allowed={allowed}')

    report = {
        'ok': len(failures) == 0,
        'failures': failures,
    }
    print(json.dumps(report, indent=2))

    if failures:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
