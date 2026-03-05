# Rust Backend

Current status:
- Implemented in Rust: `GET /query`, `GET /card`, `GET /schools`, `POST /clear-index`, `GET/POST /parser-settings`, `GET /parser-events`, `GET /documents`, `POST /delete-document`, `POST /upload-docx`, `POST /parse-uploaded-docs`, `POST /index-document`
- Parser engine: native Rust parser
- Implemented in Rust (compat/no-op): `POST /create-user`

## Pipeline binaries

The backend includes dedicated Rust binaries under `src/bin`:

- `cargo run --bin local_parser`
- `cargo run --bin scraper`
- `cargo run --bin main_pipeline` (Open Evidence scrape pipeline)
- `cargo run --bin wipe`
- `cargo run --bin logos-backend` (primary server binary)

These pipeline binaries default to direct indexing mode (`BACKEND_BRIDGE_MODE=direct`), invoking `logos-backend` with `INDEX_ONE_FILENAME` for each file. API mode remains available via `BACKEND_BRIDGE_MODE=api` + `BACKEND_API_URL`.

Benchmark native Rust parser throughput on local `.docx` files:

```bash
RUN_PARSER_BENCH=1 PARSER_BENCH_LIMIT=20 cargo run
```

- `RUN_PARSER_BENCH=1` runs benchmark mode and exits (does not start server)
- `PARSER_BENCH_LIMIT` limits number of files tested

## Run

From workspace root:

```bash
cd "./rust-backend"
PORT=5002 cargo run
```

If you want to run it against the dev local docs used by `start-rust.sh`:

```bash
cd "./rust-backend"
LOCAL_DOCS_FOLDER="../local_docs_dev" \
LOCAL_INDEX_PATH="../local_docs_dev/cards_index.json" \
PARSER_SETTINGS_PATH="../local_docs_dev/parser_settings.json" \
PARSER_EVENTS_PATH="../local_docs_dev/parser_events.jsonl" \
PORT=5002 cargo run
```

Then point the frontend to Rust API:

```bash
cd "./logos-web"
NEXT_PUBLIC_API_URL="http://localhost:5002" yarn dev
```

## Docker

The Rust backend now owns container packaging via `rust-backend/Dockerfile`.

Build from workspace root:

```bash
docker build -f rust-backend/Dockerfile -t logos-backend:latest rust-backend
```

Run:

```bash
docker run --rm -p 5002:5002 logos-backend:latest
```

## Run pipeline binaries

From `rust-backend`:

```bash
# wipe index
cargo run --bin wipe

# local parser pipeline (direct mode default)
BACKEND_BRIDGE_MODE=direct BACKEND_EXECUTABLE="./target/debug/logos-backend" cargo run --bin local_parser

# scraper pipeline (direct mode default)
BACKEND_BRIDGE_MODE=direct BACKEND_EXECUTABLE="./target/debug/logos-backend" cargo run --bin scraper

# main pipeline defaults (Open Evidence, direct mode)
BACKEND_BRIDGE_MODE=direct BACKEND_EXECUTABLE="./target/debug/logos-backend" cargo run --bin main_pipeline

# Optional API bridge fallback
BACKEND_BRIDGE_MODE=api BACKEND_API_URL="http://127.0.0.1:5002" cargo run --bin scraper

# serve API on PORT
PORT=5002 cargo run --bin logos-backend
```
