use logos_backend::app_config::BackendPaths;
use logos_core::QueryParams;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Mutex, RwLock};
use std::time::Instant;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Listener, Manager};
use url::Url;

const APP_BACKEND_PORT: u16 = 5501;

#[derive(Default)]
struct BackendState {
    child: Mutex<Option<Child>>,
}

#[derive(Default)]
struct CachedIndexState {
    cards: Vec<logos_core::Card>,
    loaded_at: Option<Instant>,
    index_path: Option<PathBuf>,
}

impl CachedIndexState {
    fn is_valid(&self, path: &Path) -> bool {
        if self.index_path.as_deref() != Some(path) {
            return false;
        }
        if let Some(loaded_at) = self.loaded_at {
            // Cache valid for 500ms (avoids repeated disk reads during rapid queries)
            loaded_at.elapsed().as_millis() < 500
        } else {
            false
        }
    }

    fn refresh(&mut self, path: &Path) {
        self.cards = logos_core::load_cards(path);
        self.loaded_at = Some(Instant::now());
        self.index_path = Some(path.to_path_buf());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ParserSettings {
    use_parallel_processing: bool,
    parser_card_workers: u32,
    local_parser_file_workers: u32,
    flush_enabled: bool,
    flush_every_docs: u32,
}

impl ParserSettings {
    fn defaults() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|value| value.get() as u32)
            .unwrap_or(1)
            .max(1);

        Self {
            use_parallel_processing: true,
            parser_card_workers: cpus,
            local_parser_file_workers: cpus.min(8),
            flush_enabled: true,
            flush_every_docs: 250,
        }
    }

    fn normalize(input: Value) -> Self {
        let defaults = Self::defaults();
        let max_workers = std::thread::available_parallelism()
            .map(|value| value.get() as u32)
            .unwrap_or(1)
            .max(1);
        let object = input.as_object();

        let parse_bool = |key: &str, fallback: bool| -> bool {
            match object.and_then(|obj| obj.get(key)) {
                Some(Value::Bool(value)) => *value,
                Some(Value::String(value)) => match value.trim().to_ascii_lowercase().as_str() {
                    "1" | "true" | "yes" | "on" => true,
                    "0" | "false" | "no" | "off" => false,
                    _ => fallback,
                },
                Some(Value::Number(value)) => value.as_i64().unwrap_or(0) != 0,
                _ => fallback,
            }
        };

        let parse_u32 = |key: &str, fallback: u32, min: u32, max: u32| -> u32 {
            match object
                .and_then(|obj| obj.get(key))
                .and_then(|value| value.as_u64())
            {
                Some(value) => (value as u32).clamp(min, max),
                None => fallback.clamp(min, max),
            }
        };

        Self {
            use_parallel_processing: parse_bool("use_parallel_processing", defaults.use_parallel_processing),
            parser_card_workers: parse_u32(
                "parser_card_workers",
                defaults.parser_card_workers,
                1,
                max_workers,
            ),
            local_parser_file_workers: parse_u32(
                "local_parser_file_workers",
                defaults.local_parser_file_workers,
                1,
                max_workers,
            ),
            flush_enabled: parse_bool("flush_enabled", defaults.flush_enabled),
            flush_every_docs: parse_u32("flush_every_docs", defaults.flush_every_docs, 1, u32::MAX),
        }
    }
}

#[tauri::command]
fn app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
fn read_json_file(path: String) -> Result<Value, String> {
    let content = fs::read_to_string(path).map_err(|err| err.to_string())?;
    serde_json::from_str::<Value>(&content).map_err(|err| err.to_string())
}

#[tauri::command]
fn write_json_file(path: String, value: Value) -> Result<(), String> {
    let destination = PathBuf::from(path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let payload = serde_json::to_string_pretty(&value).map_err(|err| err.to_string())?;
    fs::write(destination, payload).map_err(|err| err.to_string())
}

fn resolve_local_docs_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let documents = app
        .path()
        .document_dir()
        .map_err(|err| err.to_string())?;

    Ok(documents.join("Logos Continuum").join("local_docs"))
}

fn backend_executable_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let binary_name = if cfg!(target_os = "windows") {
        "logos-backend.exe"
    } else {
        "logos-backend"
    };

    if cfg!(debug_assertions) {
        let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        return Ok(project_dir
            .join("..")
            .join("..")
            .join("rust-backend")
            .join("target")
            .join("release")
            .join(binary_name));
    }

    app.path()
        .resource_dir()
        .map_err(|err| err.to_string())
        .map(|dir| dir.join("binaries").join(binary_name))
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    Ok(())
}

fn local_backend_paths(app: &tauri::AppHandle) -> Result<BackendPaths, String> {
    let local_docs = resolve_local_docs_dir(app)?;
    fs::create_dir_all(&local_docs).map_err(|err| err.to_string())?;
    Ok(BackendPaths::from_local_docs_folder(local_docs))
}

fn load_parser_settings(path: &Path) -> ParserSettings {
    let content = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(_) => return ParserSettings::defaults(),
    };

    if content.trim().is_empty() {
        return ParserSettings::defaults();
    }

    let parsed = serde_json::from_str::<Value>(&content).unwrap_or(Value::Null);
    ParserSettings::normalize(parsed)
}

fn tail_parser_events(path: &Path, limit: usize) -> Vec<Value> {
    let content = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .rev()
        .take(limit)
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn get_cached_cards(
    app: &tauri::AppHandle,
    cache: &RwLock<CachedIndexState>,
) -> Result<Vec<logos_core::Card>, String> {
    let backend_paths = local_backend_paths(app)?;
    let index_path = &backend_paths.local_index_path;

    // Try to read from cache first
    {
        let guard = cache.read().map_err(|e| e.to_string())?;
        if guard.is_valid(index_path) {
            return Ok(guard.cards.clone());
        }
    }

    // Cache miss or expired - refresh
    {
        let mut guard = cache.write().map_err(|e| e.to_string())?;
        // Double-check after acquiring write lock
        if !guard.is_valid(index_path) {
            guard.refresh(index_path);
        }
        Ok(guard.cards.clone())
    }
}

fn invalidate_cache(cache: &RwLock<CachedIndexState>) {
    if let Ok(mut guard) = cache.write() {
        guard.loaded_at = None;
    }
}

fn sanitize_filename(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "uploaded.docx".to_string()
    } else {
        trimmed
    }
}

fn dedupe_target_path(dir: &Path, filename: &str) -> PathBuf {
    let base = Path::new(filename)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("upload")
        .to_string();
    let ext = Path::new(filename)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{}", value))
        .unwrap_or_else(|| ".docx".to_string());

    let mut candidate = dir.join(format!("{}{}", base, ext));
    let mut suffix = 1;
    while candidate.exists() {
        candidate = dir.join(format!("{}-{}{}", base, suffix, ext));
        suffix += 1;
    }
    candidate
}

fn list_uploaded_docx_filenames(local_docs_folder: &Path) -> Vec<String> {
    let upload_dir = local_docs_folder.join("uploaded_docs");
    if !upload_dir.exists() {
        return Vec::new();
    }

    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(&upload_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name.starts_with("~$") || !name.to_ascii_lowercase().ends_with(".docx") {
                continue;
            }
            names.push(name.to_string());
        }
    }

    names.sort();
    names
}

fn ensure_backend_running(app: &tauri::AppHandle, state: &BackendState) -> Result<u16, String> {
    {
        let child_guard = state.child.lock().map_err(|err| err.to_string())?;
        if child_guard.is_some() {
            return Ok(APP_BACKEND_PORT);
        }
    }

    let backend_paths = local_backend_paths(app)?;
    let executable = backend_executable_path(app)?;

    if !executable.exists() {
        return Err(format!("backend executable not found at {}", executable.display()));
    }

    let mut command = Command::new(executable);
    command.env("PORT", APP_BACKEND_PORT.to_string());
    command.env("LOCAL_DOCS_FOLDER", &backend_paths.local_docs_folder);
    command.env("LOCAL_INDEX_PATH", &backend_paths.local_index_path);
    command.env("INDEX_CACHE_PATH", &backend_paths.index_cache_path);
    command.env("PARSER_SETTINGS_PATH", &backend_paths.parser_settings_path);
    command.env("PARSER_EVENTS_PATH", &backend_paths.parser_events_path);

    let child = command.spawn().map_err(|err| err.to_string())?;
    let mut child_guard = state.child.lock().map_err(|err| err.to_string())?;
    *child_guard = Some(child);

    Ok(APP_BACKEND_PORT)
}

fn kill_backend_process(state: &BackendState) -> Result<(), String> {
    let mut child_guard = state.child.lock().map_err(|err| err.to_string())?;
    if let Some(mut child) = child_guard.take() {
        child.kill().map_err(|err| err.to_string())?;
    }

    Ok(())
}

#[tauri::command]
fn start_backend(app: tauri::AppHandle, state: tauri::State<BackendState>) -> Result<u16, String> {
    ensure_backend_running(&app, &state)
}

#[tauri::command]
fn get_parser_settings(app: tauri::AppHandle) -> Result<Value, String> {
    let backend_paths = local_backend_paths(&app)?;
    let settings = load_parser_settings(&backend_paths.parser_settings_path);
    Ok(serde_json::json!({ "settings": settings }))
}

#[tauri::command]
fn update_parser_settings(app: tauri::AppHandle, settings: Value) -> Result<Value, String> {
    let backend_paths = local_backend_paths(&app)?;
    let normalized = ParserSettings::normalize(settings);

    ensure_parent_dir(&backend_paths.parser_settings_path)?;
    let serialized = serde_json::to_string_pretty(&normalized).map_err(|err| err.to_string())?;
    fs::write(&backend_paths.parser_settings_path, serialized).map_err(|err| err.to_string())?;

    Ok(serde_json::json!({ "ok": true, "settings": normalized }))
}

#[tauri::command]
fn get_parser_events(app: tauri::AppHandle, limit: Option<usize>) -> Result<Value, String> {
    let backend_paths = local_backend_paths(&app)?;
    let effective_limit = limit.unwrap_or(120).clamp(1, 500);
    let events = tail_parser_events(&backend_paths.parser_events_path, effective_limit);
    Ok(serde_json::json!({ "events": events }))
}

#[tauri::command]
fn stop_backend(state: tauri::State<BackendState>) -> Result<(), String> {
    kill_backend_process(&state)
}

#[tauri::command]
fn query_cards(
    app: tauri::AppHandle,
    cache: tauri::State<RwLock<CachedIndexState>>,
    params: QueryParams,
) -> Result<Value, String> {
    let cards = get_cached_cards(&app, &cache)?;
    Ok(logos_core::query_cards(&cards, &params))
}

#[tauri::command]
fn get_card(
    app: tauri::AppHandle,
    cache: tauri::State<RwLock<CachedIndexState>>,
    id: String,
) -> Result<Value, String> {
    let cards = get_cached_cards(&app, &cache)?;
    Ok(logos_core::get_card(&cards, &id))
}

#[tauri::command]
fn get_schools(
    app: tauri::AppHandle,
    cache: tauri::State<RwLock<CachedIndexState>>,
) -> Result<Value, String> {
    let cards = get_cached_cards(&app, &cache)?;
    Ok(logos_core::get_schools(&cards))
}

#[tauri::command]
fn get_documents(
    app: tauri::AppHandle,
    cache: tauri::State<RwLock<CachedIndexState>>,
) -> Result<Value, String> {
    let backend_paths = local_backend_paths(&app)?;
    let cards = get_cached_cards(&app, &cache)?;
    Ok(logos_core::get_documents(&cards, &backend_paths.local_docs_folder))
}

#[tauri::command]
fn clear_index(
    app: tauri::AppHandle,
    cache: tauri::State<RwLock<CachedIndexState>>,
) -> Result<Value, String> {
    let backend_paths = local_backend_paths(&app)?;
    logos_core::clear_index_file(&backend_paths.local_index_path)?;
    invalidate_cache(&cache);
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
fn delete_document(
    app: tauri::AppHandle,
    cache: tauri::State<RwLock<CachedIndexState>>,
    filename: String,
    target: String,
) -> Result<Value, String> {
    let backend_paths = local_backend_paths(&app)?;
    let result = logos_core::delete_document(
        &backend_paths.local_index_path,
        &backend_paths.local_docs_folder,
        &filename,
        &target,
    );
    invalidate_cache(&cache);
    result
}

#[tauri::command]
async fn index_document(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    cache: tauri::State<'_, RwLock<CachedIndexState>>,
    filename: String,
) -> Result<Value, String> {
    let result = run_index_document_once(&app, &state, &filename);
    invalidate_cache(&cache);
    result
}

fn run_index_document_once(
    app: &tauri::AppHandle,
    state: &BackendState,
    filename: &str,
) -> Result<Value, String> {
    let backend_paths = local_backend_paths(&app)?;
    let executable = backend_executable_path(&app)?;

    if !executable.exists() {
        return Err(format!("backend executable not found at {}", executable.display()));
    }

    kill_backend_process(&state)?;

    let output = Command::new(executable)
        .env("PORT", APP_BACKEND_PORT.to_string())
        .env("LOCAL_DOCS_FOLDER", &backend_paths.local_docs_folder)
        .env("LOCAL_INDEX_PATH", &backend_paths.local_index_path)
        .env("INDEX_CACHE_PATH", &backend_paths.index_cache_path)
        .env("PARSER_SETTINGS_PATH", &backend_paths.parser_settings_path)
        .env("PARSER_EVENTS_PATH", &backend_paths.parser_events_path)
        .env("INDEX_ONE_FILENAME", filename)
        .output()
        .map_err(|err| err.to_string())?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let payload = serde_json::from_str::<Value>(&stdout).unwrap_or_else(|_| {
        serde_json::json!({
            "ok": false,
            "error": if stderr.is_empty() { "invalid backend response" } else { stderr.as_str() }
        })
    });

    if !output.status.success() {
        let message = payload
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or_else(|| if stderr.is_empty() { "failed to index document" } else { stderr.as_str() })
            .to_string();
        return Err(message);
    }

    Ok(payload)
}

#[tauri::command]
async fn upload_docx(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    cache: tauri::State<'_, RwLock<CachedIndexState>>,
    filename: String,
    bytes: Vec<u8>,
    parse_immediately: Option<bool>,
) -> Result<Value, String> {
    if filename.trim().is_empty() {
        return Err("filename is required".to_string());
    }

    if !filename.trim().to_ascii_lowercase().ends_with(".docx") {
        return Err("Only .docx files are supported".to_string());
    }

    let backend_paths = local_backend_paths(&app)?;
    let upload_dir = backend_paths.local_docs_folder.join("uploaded_docs");
    fs::create_dir_all(&upload_dir).map_err(|err| err.to_string())?;

    let safe_name = sanitize_filename(&filename);
    let save_path = dedupe_target_path(&upload_dir, &safe_name);
    fs::write(&save_path, bytes).map_err(|err| err.to_string())?;

    let stored_filename = save_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(&safe_name)
        .to_string();

    let parse_flag = parse_immediately.unwrap_or(true);

    if !parse_flag {
        return Ok(serde_json::json!({
            "ok": true,
            "queued": false,
            "deferred": true,
            "filename": stored_filename,
            "stored_path": save_path.to_string_lossy(),
            "cards_indexed": 0,
            "parse_ms": 0,
        }));
    }

    let index_result = run_index_document_once(&app, &state, &stored_filename)?;
    invalidate_cache(&cache);
    let cards_indexed = index_result
        .get("cards_indexed")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);

    Ok(serde_json::json!({
        "ok": true,
        "queued": false,
        "deferred": false,
        "filename": stored_filename,
        "stored_path": save_path.to_string_lossy(),
        "cards_indexed": cards_indexed,
        "parse_ms": 0,
    }))
}

#[tauri::command]
async fn parse_uploaded_docs(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    cache: tauri::State<'_, RwLock<CachedIndexState>>,
) -> Result<Value, String> {
    let backend_paths = local_backend_paths(&app)?;
    let cards = logos_core::load_cards(&backend_paths.local_index_path);
    let indexed_filenames: HashSet<String> = cards
        .iter()
        .filter_map(|card| card.get("filename").and_then(|value| value.as_str()))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect();

    let mut queued = 0_u64;
    let mut skipped = 0_u64;
    for filename in list_uploaded_docx_filenames(&backend_paths.local_docs_folder) {
        if indexed_filenames.contains(&filename.to_ascii_lowercase()) {
            skipped += 1;
            continue;
        }

        run_index_document_once(&app, &state, &filename)?;
        queued += 1;
    }

    invalidate_cache(&cache);

    Ok(serde_json::json!({
        "ok": true,
        "queued": queued,
        "skipped_already_indexed": skipped,
    }))
}

#[tauri::command]
async fn benchmark_parse_throughput(
    app: tauri::AppHandle,
    state: tauri::State<'_, BackendState>,
    _cache: tauri::State<'_, RwLock<CachedIndexState>>,
    limit: Option<usize>,
) -> Result<Value, String> {
    let backend_paths = local_backend_paths(&app)?;
    let filenames = list_uploaded_docx_filenames(&backend_paths.local_docs_folder);
    let effective_limit = limit.unwrap_or(filenames.len()).min(filenames.len());

    if effective_limit == 0 {
        return Ok(serde_json::json!({
            "ok": true,
            "files_parsed": 0,
            "total_cards": 0,
            "elapsed_ms": 0,
            "cards_per_second": 0.0,
        }));
    }

    let start = Instant::now();
    let mut total_cards = 0_u64;
    let mut files_parsed = 0_u64;

    for filename in filenames.into_iter().take(effective_limit) {
        match run_index_document_once(&app, &state, &filename) {
            Ok(result) => {
                if let Some(count) = result.get("cards_indexed").and_then(|v| v.as_u64()) {
                    total_cards += count;
                }
                files_parsed += 1;
            }
            Err(_) => continue,
        }
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let elapsed_secs = elapsed_ms as f64 / 1000.0;
    let cards_per_second = if elapsed_secs > 0.0 {
        total_cards as f64 / elapsed_secs
    } else {
        0.0
    };

    Ok(serde_json::json!({
        "ok": true,
        "files_parsed": files_parsed,
        "total_cards": total_cards,
        "elapsed_ms": elapsed_ms,
        "cards_per_second": cards_per_second,
    }))
}

#[tauri::command]
async fn check_for_updates(app: tauri::AppHandle) -> Result<Value, String> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => Ok(serde_json::json!({
            "available": true,
            "version": update.version,
            "current_version": update.current_version,
        })),
        Ok(None) => Ok(serde_json::json!({
            "available": false,
        })),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<Value, String> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => {
            let mut downloaded = 0;
            let _ = update
                .download_and_install(
                    |chunk_length, _content_length| {
                        downloaded += chunk_length;
                        eprintln!("downloaded {} bytes", downloaded);
                    },
                    || {
                        eprintln!("download finished, installing...");
                    },
                )
                .await;
            Ok(serde_json::json!({ "ok": true, "installed": true }))
        }
        Ok(None) => Ok(serde_json::json!({ "ok": true, "installed": false, "reason": "no update available" })),
        Err(e) => Err(e.to_string()),
    }
}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show_item = MenuItemBuilder::with_id("show", "Show Window").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit Logos").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&show_item)
        .separator()
        .item(&quit_item)
        .build()?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().cloned().unwrap())
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                let state = app.state::<BackendState>();
                let _ = kill_backend_process(&state);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

fn handle_deep_link(app: &tauri::AppHandle, urls: Vec<Url>) {
    for url in urls {
        let scheme = url.scheme();
        if scheme != "logos" {
            continue;
        }

        let host = url.host_str().unwrap_or("");
        let path = url.path();

        // Emit event to frontend for handling
        let _ = app.emit(
            "deep-link",
            serde_json::json!({
                "scheme": scheme,
                "host": host,
                "path": path,
                "query": url.query().unwrap_or(""),
                "full": url.to_string(),
            }),
        );

        // Show and focus the main window
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .manage(BackendState::default())
        .manage(RwLock::new(CachedIndexState::default()))
        .setup(|app| {
            // Setup system tray
            if let Err(e) = setup_tray(app) {
                eprintln!("Failed to setup tray: {}", e);
            }

            // Handle deep links
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            {
                let app_handle = app.handle().clone();
                app.listen("deep-link://new-url", move |event: tauri::Event| {
                    if let Ok(urls) = serde_json::from_str::<Vec<String>>(event.payload()) {
                        let parsed: Vec<Url> = urls
                            .into_iter()
                            .filter_map(|s| Url::parse(&s).ok())
                            .collect();
                        handle_deep_link(&app_handle, parsed);
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            read_json_file,
            write_json_file,
            start_backend,
            stop_backend,
            query_cards,
            get_card,
            get_schools,
            get_documents,
            clear_index,
            delete_document,
            index_document,
            upload_docx,
            parse_uploaded_docs,
            get_parser_settings,
            update_parser_settings,
            get_parser_events,
            benchmark_parse_throughput,
            check_for_updates,
            install_update
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if matches!(event, tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }) {
                let state = app_handle.state::<BackendState>();
                let _ = kill_backend_process(&state);
            }
        });
}
