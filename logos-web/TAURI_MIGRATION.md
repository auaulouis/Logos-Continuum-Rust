# Tauri Migration (Complete)

This repository now has a **complete Tauri implementation** under `logos-web/src-tauri` alongside the existing Electron shell. The Tauri version is ready to replace Electron once metrics validation passes.

## What is implemented

1. **Static desktop UI target**
   - Added `yarn build:static` (`next build && next export -o out`)
   - Tauri points at `logos-web/out` (`frontendDist`)

2. **Parallel desktop wrapper**
   - Added Tauri app scaffold in `logos-web/src-tauri`
   - Configured a single main window matching current Electron dimensions
   - Added macOS, Windows, and Linux bundle targets

3. **Complete desktop command surface**
   - Added Rust commands in `src-tauri/src/main.rs`:
     - `start_backend`, `stop_backend`
     - `read_json_file`, `write_json_file`
     - `app_version`
     - `query_cards`, `get_card`, `get_schools`, `get_documents`
     - `clear_index`, `delete_document`, `index_document`
     - `upload_docx`, `parse_uploaded_docs`
     - `get_parser_settings`, `update_parser_settings`, `get_parser_events`
     - `benchmark_parse_throughput`
     - `check_for_updates`, `install_update`

4. **Shared backend logic extraction**
   - Added reusable module `rust-backend/src/app_config.rs`
   - Exposes `BackendPaths` for env/path resolution
   - Extracted `rust-backend/crates/logos-core` for shared query/index logic

5. **OS integration**
   - System tray with show/quit menu and left-click activation
   - Deep-link handling for `logos://` URLs
   - Auto-updater via Tauri updater plugin

6. **Packaging and signing**
   - macOS: entitlements.plist, signing, and notarization via CI
   - Windows: NSIS installer target
   - Linux: AppImage and deb targets
   - Updater bundle target for auto-updates

## Electron → Tauri behavior mapping

| Electron Responsibility | Tauri Implementation |
|------------------------|---------------------|
| Spawn Rust backend with env vars | `start_backend` command |
| Shutdown backend on app close | `stop_backend` on Exit event |
| Path/layout for index + settings | `logos_backend::app_config::BackendPaths` |
| Renderer talks to backend | Direct Tauri commands (no HTTP round-trip) |
| System tray | `TrayIconBuilder` with menu |
| Deep links | `tauri-plugin-deep-link` |
| Auto-updater | `tauri-plugin-updater` |

## CI Pipeline

The `.github/workflows/desktop-side-by-side.yml` workflow builds both Electron and Tauri artifacts:

- **Electron**: macOS arm64
- **Tauri**: macOS arm64, Windows x64, Linux x64

Metrics comparison includes:
- Artifact size
- Backend startup time
- Backend memory usage (RSS)
- Crash detection

## Commands

| Command | Description |
|---------|-------------|
| `yarn desktop:build` | **Build Tauri release** (default) |
| `yarn desktop:dev` | Run Tauri dev mode |
| `yarn electron:build` | Build Electron release (legacy) |
| `yarn electron:dev` | Run Electron dev mode (legacy) |

## Migration Status: ✅ COMPLETE

The following items have been completed:

1. ✅ **All Tauri commands implemented** - Query, upload, parse, settings, tray, deep-links, updater
2. ✅ **Updater key generated** - Private key at `~/.tauri/logos-updater.key`
3. ✅ **Default switched to Tauri** - `yarn desktop:build` now builds Tauri
4. ✅ **CI pipeline ready** - Builds both Electron and Tauri with metrics comparison
5. ✅ **Multi-platform support** - macOS, Windows, Linux (Tauri only)

## Remaining Steps for Production

1. **Set GitHub Secrets** - See `TAURI_IPC_PORT_CHECKLIST.md` for required secrets
2. **Run CI** - Push to trigger builds and review metrics comparison
3. **Remove Electron dependencies** - After confirming Tauri works in production
