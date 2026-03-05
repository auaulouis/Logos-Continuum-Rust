# Electron → Tauri Port Checklist

This checklist maps desktop/main-process responsibilities to explicit Tauri command targets.

## Current source of truth

- Electron main process: `logos-web/electron/main.js`
- Initial Tauri commands: `logos-web/src-tauri/src/main.rs`

## Port status

### Phase A — Core lifecycle (in progress)

- [x] `startParserBackend()` → `start_backend`
- [x] backend env/path wiring (`LOCAL_*`, `PARSER_*`) moved to shared Rust config module (`rust-backend/src/app_config.rs`)
- [x] frontend desktop bootstrap calls `start_backend` in `pages/_app.tsx`
- [x] `window-all-closed` backend shutdown parity (`stop_backend` on app exit event)

### Phase B — File/settings/native APIs

- [x] Add generic JSON file read command: `read_json_file`
- [x] Add generic JSON file write command: `write_json_file`
- [x] Replace parser-settings HTTP usage with direct command pair:
  - [x] `get_parser_settings`
  - [x] `update_parser_settings`
- [x] Replace parser-events polling HTTP with direct command:
  - [x] `get_parser_events`

### Phase C — Query/index operations (shared logic target)

- [x] Extract reusable query/index engine crate from `rust-backend/src/main.rs` (first thin slice in `rust-backend/crates/logos-core`)
- [x] Add Tauri commands backed by shared crate (no localhost HTTP):
  - [x] `query_cards`
  - [x] `get_card`
  - [x] `get_schools`
  - [x] `get_documents`
  - [x] `delete_document`
  - [x] `index_document`
  - [x] `clear_index`

### Phase D — Upload/parse flow

- [x] Design Tauri file ingest API to replace multipart `/upload-docx` *(temporary backend bridge inside Tauri command)*
- [x] Add command for parse queueing equivalent to `/parse-uploaded-docs` *(temporary backend bridge inside Tauri command)*
- [x] Preserve parse worker settings parity and benchmark throughput
  - [x] `benchmark_parse_throughput` command for measuring parse speed

### Phase E — Packaging and OS integration

- [x] Tray behavior parity
  - [x] System tray icon with show/quit menu
  - [x] Left-click to show window
- [x] Deep-link handling parity
  - [x] `logos://` URL scheme registration
  - [x] Deep-link event emission to frontend
- [x] Auto-update strategy parity (Tauri updater vs current Electron approach)
  - [x] `check_for_updates` command
  - [x] `install_update` command
  - [x] Tauri updater plugin configuration
- [x] macOS signing/notarization pipeline parity
  - [x] `entitlements.plist` for macOS capabilities
  - [x] CI workflow with Apple certificate import
  - [x] Notarization step in CI

## Side-by-side beta metrics

- [x] CI artifacts for both Electron and Tauri
  - [x] macOS arm64 (both)
  - [x] Windows x64 (Tauri only)
  - [x] Linux x64 (Tauri only)
- [x] Startup time comparison (in CI metrics summary)
- [x] RAM baseline comparison (in CI metrics summary)
- [x] Crash/exit stability comparison (backend_crashed_before_ready metric)
- [ ] Switch default desktop target after parity threshold is met

## Required Secrets for CI

The following GitHub secrets must be configured for full CI functionality:

### Tauri Updater
- `TAURI_SIGNING_PRIVATE_KEY` - Contents of `~/.tauri/logos-updater.key` (base64 minisign key)
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` - Empty string (key generated without password)

**Local key files:**
- Private key: `~/.tauri/logos-updater.key`
- Public key: `~/.tauri/logos-updater.key.pub`

**To regenerate keys (invalidates existing updates):**
```bash
cd logos-web && yarn tauri signer generate --password "" -w ~/.tauri/logos-updater.key
```

### macOS Signing/Notarization
- `APPLE_CERTIFICATE` - Base64-encoded .p12 certificate
- `APPLE_CERTIFICATE_PASSWORD` - Password for the certificate
- `APPLE_SIGNING_IDENTITY` - Signing identity (e.g., "Developer ID Application: ...")
- `APPLE_ID` - Apple ID email
- `APPLE_PASSWORD` - App-specific password
- `APPLE_TEAM_ID` - Apple Developer Team ID

### Setting Up GitHub Secrets

1. Go to GitHub repo → Settings → Secrets and variables → Actions
2. Add the following repository secrets:

| Secret Name | Value |
|-------------|-------|
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of `~/.tauri/logos-updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | (empty) |
| `APPLE_CERTIFICATE` | `base64 -i certificate.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | Your certificate password |
| `APPLE_SIGNING_IDENTITY` | e.g., `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | Your Apple ID email |
| `APPLE_PASSWORD` | App-specific password from appleid.apple.com |
| `APPLE_TEAM_ID` | Your 10-character Team ID |
