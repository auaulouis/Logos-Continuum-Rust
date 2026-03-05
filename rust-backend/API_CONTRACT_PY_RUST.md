# Python ↔ Rust API Contract (Frozen)

This contract freezes request/response parity between:
- Python: `verbatim-parser /api.py`
- Rust: `rust-backend/src/main.rs`

Date frozen: 2026-03-04

## Global semantics

- Success responses are JSON.
- Error responses are JSON.
- Standard error object shape for validation/runtime failures:
  - `{ "error": string }`
- `GET /query` pagination is cursor-based:
  - request: `cursor` + `limit`
  - response: `cursor`, `has_more`, `total_count`, `count_is_partial`
- `limit` on query is clamped to `1..30`.
- `limit` on parser-events is clamped to `1..500` (default `120`).
- `target` in delete is one of: `index | folder`.

## Endpoint contract

### `GET /health` (Rust only)
- `200`: `{ "ok": true }`

### `GET /query`
Query params:
- `search`, `cursor`, `start_date`, `end_date`, `exclude_sides`, `exclude_division`, `exclude_schools`, `exclude_years`, `sort_by`, `cite_match`, `limit`, `match_mode`

Response (`200`):
- `count: number`
- `results: CardSearchResult[]`
- `cursor: number`
- `total_count: number`
- `has_more: boolean`
- `count_is_partial: boolean`

### `GET /card`
Query params: `id`

Response (`200`):
- found: full card object
- missing: `null`

### `GET /schools`
Response (`200`): `{ "colleges": string[] }`

### `POST /clear-index`
- `200`: `{ "ok": true }`
- `500`: `{ "error": string }`

### `GET /parser-settings`
- `200`: `{ "settings": ParserSettings }`

### `POST /parser-settings`
Body: parser settings object (partial accepted)

- `200`: `{ "ok": true, "settings": ParserSettings }`
- `500`: `{ "error": string }` on serialization or write failures

### `GET /parser-events`
Query params: `limit`

- `200`: `{ "events": object[] }`

### `GET /documents`
- `200`: `{ "documents": DocumentSummary[] }`

`DocumentSummary`:
- `filename: string`
- `cards_indexed: number`
- `in_index: boolean`
- `in_folder: boolean`
- `folder_path: string | null`

### `POST /delete-document`
Body: `{ "filename": string, "target": "index" | "folder" }`

- `200`: `{ "ok": true, "removed_cards": number, "removed_from_folder": boolean, "deleted_path": string | null }`
- `400`: `{ "error": "filename is required" }`
- `400`: `{ "error": "target must be either 'index' or 'folder'" }`
- `404`: `{ "ok": false, "removed_cards": 0, "removed_from_folder": false, "deleted_path": null, "message": "Document not found for selected target" }`
- `500`: `{ "error": string }`

### `POST /upload-docx`
Multipart fields:
- `file` (required)
- `parse` (optional bool-like)

Success (`200`) fields:
- `ok`, `queued`, `filename`, `stored_path`, `cards_indexed`, `parse_ms`
- `deferred` appears when parse is disabled

Errors:
- `400`: `{ "error": "No file uploaded" }`
- `400`: `{ "error": "Only .docx files are supported" }`
- `400|500`: `{ "error": string }` for malformed multipart / storage failures

### `POST /parse-uploaded-docs`
- `200`: `{ "ok": true, "queued": number, "skipped_already_indexed": number }`

### `POST /index-document`
Body: `{ "filename": string }`

- `200`: `{ "ok": true, "filename": string, "cards_indexed": number }`
- `400`: `{ "error": "filename is required" }`
- `404`: `{ "error": "Document file not found in uploaded_docs" }`
- `500`: `{ "error": "Failed to index <filename>: <message>" }`

### `POST /create-user`
- Rust: `200 { "ok": true, "noop": true }`
- Python: not implemented

## Normalization parity rules

- Parser worker counts clamp to `1..cpu_count`.
- `flush_every_docs` clamps to minimum `1`.
- Boolean parser settings accept: bool/string/number
  - true forms: `true|1|yes|on`
  - false forms: `false|0|no|off`
- `exclude_division` compares leading segment before `-`.

## Known tolerated behavior differences

- Rust query can return `count_is_partial=true` during incremental scan for cursor windows.
- JSON object key order is not contract-significant.
