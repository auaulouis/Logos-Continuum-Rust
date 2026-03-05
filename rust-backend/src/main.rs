use std::collections::{hash_map::DefaultHasher, BTreeSet, HashMap, HashSet};
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Multipart, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Datelike, NaiveDate};
use once_cell::sync::Lazy;
use quick_xml::events::Event;
use quick_xml::Reader;
use rayon::prelude::*;
use regex::Regex;
use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, Mutex};
use tower_http::cors::{Any, CorsLayer};
use tracing::info;
use walkdir::WalkDir;
use zip::ZipArchive;
use logos_backend::app_config::BackendPaths;
use logos_core as shared_core;
use logos_core::QueryParams;

type Card = Map<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct IndexSignature {
    size: u64,
    modified_ns: u128,
}

impl IndexSignature {
    fn empty() -> Self {
        Self { size: 0, modified_ns: 0 }
    }
}

#[derive(Debug)]
struct CardsCache {
    signature: IndexSignature,
    cards: Arc<Vec<Card>>,
    query_cards: Arc<Vec<QueryCardCache>>,
    term_index: Arc<HashMap<String, RoaringBitmap>>,
    id_lookup: Arc<HashMap<String, usize>>,
    filename_counts: Arc<HashMap<String, (String, u64)>>,
    schools: Arc<BTreeSet<String>>,
    indexed_filenames: Arc<HashSet<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueryCardCache {
    filename_lc: String,
    division_lc: String,
    year_lc: String,
    school_lc: String,
    cite_lc: String,
    search_blob_lc: String,
    cite_ts: Option<i64>,
}

#[derive(Clone)]
struct CardsCacheSnapshot {
    signature: IndexSignature,
    cards: Arc<Vec<Card>>,
    query_cards: Arc<Vec<QueryCardCache>>,
    term_index: Arc<HashMap<String, RoaringBitmap>>,
    id_lookup: Arc<HashMap<String, usize>>,
    filename_counts: Arc<HashMap<String, (String, u64)>>,
    schools: Arc<BTreeSet<String>>,
    indexed_filenames: Arc<HashSet<String>>,
}

#[derive(Debug)]
struct ParseJob {
    doc_path: PathBuf,
    stored_filename: String,
}

#[derive(Debug, Clone)]
struct QueryStateEntry {
    signature: IndexSignature,
    candidate_indices: Vec<usize>,
    matched_indices: Vec<usize>,
    scan_index: usize,
    exhausted: bool,
    last_used_ms: u128,
}

impl QueryStateEntry {
    fn new(signature: IndexSignature, candidate_indices: Vec<usize>) -> Self {
        Self {
            signature,
            candidate_indices,
            matched_indices: Vec::new(),
            scan_index: 0,
            exhausted: false,
            last_used_ms: now_millis(),
        }
    }
}

#[derive(Debug, Clone)]
struct QueryResultCacheEntry {
    signature: IndexSignature,
    payload: Value,
    expires_at_ms: u128,
    last_used_ms: u128,
}

const QUERY_STATE_SHARDS: usize = 16;
const QUERY_RESULT_SHARDS: usize = 16;
const MAX_QUERY_STATE_ENTRIES_PER_SHARD: usize = 16;
const MAX_QUERY_RESULT_ENTRIES_PER_SHARD: usize = 64;
const QUERY_RESULT_CACHE_TTL_MS: u128 = 1200;

#[derive(Clone)]
struct AppState {
    local_docs_folder: PathBuf,
    local_index_path: PathBuf,
    index_cache_path: PathBuf,
    parser_settings_path: PathBuf,
    parser_events_path: PathBuf,
    index_lock: Arc<Mutex<()>>,
    active_upload_parses: Arc<Mutex<HashSet<String>>>,
    parse_job_sender: mpsc::Sender<ParseJob>,
    cards_cache: Arc<Mutex<CardsCache>>,
    query_state_shards: Arc<Vec<Mutex<HashMap<String, QueryStateEntry>>>>,
    query_result_shards: Arc<Vec<Mutex<HashMap<String, QueryResultCacheEntry>>>>,
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
        let cpus = num_cpus::get().max(1) as u32;
        Self {
            use_parallel_processing: true,
            parser_card_workers: cpus.min(4),
            local_parser_file_workers: cpus.min(8),
            flush_enabled: true,
            flush_every_docs: 250,
        }
    }

    fn normalize(input: Value) -> Self {
        let defaults = Self::defaults();
        let max_workers = num_cpus::get().max(1) as u32;
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

#[derive(Debug, Deserialize)]
struct CardParams {
    id: String,
}

#[derive(Debug, Deserialize)]
struct EventsParams {
    #[serde(default = "default_events_limit")]
    limit: usize,
}

fn default_events_limit() -> usize {
    120
}

#[derive(Debug, Deserialize)]
struct DeleteDocumentRequest {
    filename: String,
    target: String,
}

#[derive(Debug, Deserialize)]
struct IndexDocumentRequest {
    filename: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let backend_paths = BackendPaths::from_env();
    let local_docs_folder = backend_paths.local_docs_folder;
    let local_index_path = backend_paths.local_index_path;
    let index_cache_path = backend_paths.index_cache_path;
    let parser_settings_path = backend_paths.parser_settings_path;
    let parser_events_path = backend_paths.parser_events_path;

    ensure_parent_dir(&local_index_path);
    ensure_parent_dir(&index_cache_path);
    ensure_parent_dir(&parser_settings_path);
    ensure_parent_dir(&parser_events_path);

    let upload_parse_workers = env::var("UPLOAD_PARSE_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(|| num_cpus::get().clamp(1, 4))
        .max(1);
    let queue_capacity = (upload_parse_workers * 16).max(16);
    let (parse_job_sender, parse_job_receiver) = mpsc::channel::<ParseJob>(queue_capacity);
    let parse_job_receiver = Arc::new(Mutex::new(parse_job_receiver));

    let state = Arc::new(AppState {
        local_docs_folder,
        local_index_path,
        index_cache_path,
        parser_settings_path,
        parser_events_path,
        index_lock: Arc::new(Mutex::new(())),
        active_upload_parses: Arc::new(Mutex::new(HashSet::new())),
        parse_job_sender,
        cards_cache: Arc::new(Mutex::new(CardsCache {
            signature: IndexSignature::empty(),
            cards: Arc::new(Vec::new()),
            query_cards: Arc::new(Vec::new()),
            term_index: Arc::new(HashMap::new()),
            id_lookup: Arc::new(HashMap::new()),
            filename_counts: Arc::new(HashMap::new()),
            schools: Arc::new(BTreeSet::new()),
            indexed_filenames: Arc::new(HashSet::new()),
        })),
        query_state_shards: Arc::new(empty_query_state_shards()),
        query_result_shards: Arc::new(empty_query_result_shards()),
    });

    start_parse_workers(state.clone(), parse_job_receiver, upload_parse_workers);
    warm_index_caches_on_startup(&state).await;

    if let Ok(filename) = env::var("INDEX_ONE_FILENAME") {
        let filename = filename.trim().to_string();
        if filename.is_empty() {
            println!("{}", json!({ "ok": false, "error": "filename is required" }));
            process::exit(1);
        }

        match index_document_once(state.clone(), &filename).await {
            Ok(payload) => {
                println!("{}", payload);
                process::exit(0);
            }
            Err(error) => {
                println!("{}", json!({ "ok": false, "error": error }));
                process::exit(1);
            }
        }
    }

    if env::var("RUN_PARSER_BENCH").ok().as_deref() == Some("1") {
        run_parser_benchmark(state.clone()).await;
        return;
    }

    let app = Router::new()
        .route("/health", get(health))
        .route("/query", get(query_cards))
        .route("/card", get(get_card))
        .route("/schools", get(get_schools))
        .route("/clear-index", post(clear_index))
        .route("/parser-settings", get(get_parser_settings).post(update_parser_settings))
        .route("/parser-events", get(get_parser_events))
        .route("/documents", get(get_documents))
        .route("/delete-document", post(delete_document))
        .route("/upload-docx", post(upload_docx))
        .route("/parse-uploaded-docs", post(parse_uploaded_docs))
        .route("/index-document", post(index_document))
        .route("/create-user", post(create_user))
        .with_state(state)
        .layer(axum::extract::DefaultBodyLimit::disable())
        .layer(CorsLayer::new().allow_origin(Any).allow_headers(Any).allow_methods(Any));

    let port = env::var("PORT").ok().and_then(|value| value.parse::<u16>().ok()).unwrap_or(5002);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("failed to bind listener");

    info!("Rust backend listening on http://localhost:{}", port);
    axum::serve(listener, app).await.expect("server failed");
}

async fn index_document_once(state: Arc<AppState>, filename: &str) -> Result<Value, String> {
    let file_item = find_uploaded_docx_file(&state.local_docs_folder, filename)
        .ok_or_else(|| "Document file not found in uploaded_docs".to_string())?;

    let parsed_cards = parse_docx_cards(&state, &file_item.absolute_path, &file_item.filename).await?;
    let _ = append_cards_to_index(&state, &parsed_cards, true).await?;

    Ok(json!({
        "ok": true,
        "filename": file_item.filename,
        "cards_indexed": parsed_cards.len(),
    }))
}

async fn health() -> impl IntoResponse {
    Json(json!({ "ok": true }))
}

async fn query_cards(
    State(state): State<Arc<AppState>>,
    Query(params): Query<QueryParams>,
) -> impl IntoResponse {
    let snapshot = get_cards_cache_snapshot(&state).await;
    let cards_ref = snapshot.cards.as_ref();
    let query_cards_ref = snapshot.query_cards.as_ref();
    let signature = snapshot.signature;

    let safe_limit = params.limit.clamp(1, 30);
    let offset = params.cursor;

    let (terms, phrases) = shared_core::split_query_terms(&params.search);
    let (term_groups, phrase_groups) = expand_query_with_synonyms(&terms, &phrases);
    let candidate_indices = derive_candidate_indices(
        cards_ref.len(),
        &term_groups,
        snapshot.term_index.as_ref(),
    );

    let excluded_sides: HashSet<String> = shared_core::split_csv(&params.exclude_sides).into_iter().collect();
    let excluded_divisions: HashSet<String> = shared_core::split_csv(&params.exclude_division)
        .into_iter()
        .map(|value| value.split('-').next().unwrap_or("").trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    let excluded_years: HashSet<String> = shared_core::split_csv(&params.exclude_years).into_iter().collect();
    let excluded_schools: HashSet<String> = shared_core::split_csv(&params.exclude_schools).into_iter().collect();

    let normalized_match_mode = params.match_mode.trim().to_ascii_lowercase();
    let normalized_sort_by = params.sort_by.trim().to_ascii_lowercase();
    let cite_match = params.cite_match.trim().to_ascii_lowercase();
    let result_cache_key = normalized_query_result_key(
        &params,
        &normalized_sort_by,
        &cite_match,
        &normalized_match_mode,
        safe_limit,
        offset,
    );

    if let Some(cached_payload) = get_cached_query_response(&state, &result_cache_key, &signature).await {
        return Json(cached_payload);
    }

    let start_ts = shared_core::to_unix_timestamp(&params.start_date);
    let end_ts = shared_core::to_unix_timestamp(&params.end_date);

    if normalized_sort_by == "date" {
        let mut matched_indices: Vec<usize> = Vec::new();

        for card_index in candidate_indices.iter().copied() {
            let card = &cards_ref[card_index];
            let query_card = &query_cards_ref[card_index];
            if !card_matches(
                card,
                query_card,
                &term_groups,
                &phrase_groups,
                &excluded_sides,
                &excluded_divisions,
                &excluded_years,
                &excluded_schools,
                start_ts,
                end_ts,
                &cite_match,
                &normalized_match_mode,
            ) {
                continue;
            }
            matched_indices.push(card_index);
        }

        matched_indices.sort_by(|a, b| {
            let a_ts = query_cards_ref[*a].cite_ts.unwrap_or(0);
            let b_ts = query_cards_ref[*b].cite_ts.unwrap_or(0);
            b_ts.cmp(&a_ts)
        });

        let total_count = matched_indices.len();
        let page_results: Vec<Value> = matched_indices
            .into_iter()
            .skip(offset)
            .take(safe_limit)
            .filter_map(|index| cards_ref.get(index).map(shared_core::card_to_search_result))
            .collect();

        let cursor = offset + page_results.len();
        let has_more = cursor < total_count;

        let payload = json!({
            "count": page_results.len(),
            "results": page_results,
            "cursor": cursor,
            "total_count": total_count,
            "has_more": has_more,
            "count_is_partial": false
        });

        store_cached_query_response(&state, &result_cache_key, signature, payload.clone()).await;
        return Json(payload);
    }

    let key = query_state_key(
        &params,
        &normalized_sort_by,
        &cite_match,
        &normalized_match_mode,
    );
    let target = offset + safe_limit;
    let initial_candidates = candidate_indices;

    let (page_indices, total_count, exhausted, has_more, cursor) = {
        let shard_idx = shard_index_for_key(&key, state.query_state_shards.len());
        let mut cache = state.query_state_shards[shard_idx].lock().await;

        let entry = cache
            .entry(key)
            .or_insert_with(|| QueryStateEntry::new(signature.clone(), initial_candidates.clone()));

        if entry.signature != signature {
            *entry = QueryStateEntry::new(signature.clone(), initial_candidates.clone());
        }

        entry.last_used_ms = now_millis();

        while entry.matched_indices.len() < target && !entry.exhausted {
            while entry.scan_index < entry.candidate_indices.len() && entry.matched_indices.len() < target {
                let card_index = entry.candidate_indices[entry.scan_index];
                entry.scan_index += 1;

                let card = &cards_ref[card_index];
                let query_card = &query_cards_ref[card_index];
                if card_matches(
                    card,
                    query_card,
                    &term_groups,
                    &phrase_groups,
                    &excluded_sides,
                    &excluded_divisions,
                    &excluded_years,
                    &excluded_schools,
                    start_ts,
                    end_ts,
                    &cite_match,
                    &normalized_match_mode,
                ) {
                    entry.matched_indices.push(card_index);
                }
            }

            if entry.scan_index >= entry.candidate_indices.len() {
                entry.exhausted = true;
            }
        }

        let end = entry.matched_indices.len().min(offset + safe_limit);
        let page_indices = if offset < end {
            entry.matched_indices[offset..end].to_vec()
        } else {
            Vec::new()
        };
        let cursor = offset + page_indices.len();
        let has_more = entry.matched_indices.len() > cursor || !entry.exhausted;
        let total_count = entry.matched_indices.len();
        let exhausted = entry.exhausted;

        prune_query_state_cache(&mut cache);

        (page_indices, total_count, exhausted, has_more, cursor)
    };

    let page_results: Vec<Value> = page_indices
        .into_iter()
        .filter_map(|index| cards_ref.get(index).map(shared_core::card_to_search_result))
        .collect();

    let payload = json!({
        "count": page_results.len(),
        "results": page_results,
        "cursor": cursor,
        "total_count": total_count,
        "has_more": has_more,
        "count_is_partial": !exhausted
    });

    store_cached_query_response(&state, &result_cache_key, signature, payload.clone()).await;
    Json(payload)
}

async fn get_card(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CardParams>,
) -> impl IntoResponse {
    let cache = get_cards_cache_snapshot(&state).await;
    let card = cache
        .id_lookup
        .get(params.id.as_str())
        .and_then(|index| cache.cards.get(*index))
        .cloned()
        .map(Value::Object)
        .unwrap_or(Value::Null);

    Json(card)
}

async fn get_schools(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cache = get_cards_cache_snapshot(&state).await;
    Json(json!({ "colleges": cache.schools.iter().cloned().collect::<Vec<_>>() }))
}

async fn clear_index(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Err(error) = shared_core::clear_index_file(&state.local_index_path) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("failed to clear index: {}", error) })),
        )
            .into_response();
    }

    refresh_index_caches(&state).await;

    Json(json!({ "ok": true })).into_response()
}

async fn get_parser_settings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let settings = load_parser_settings(&state.parser_settings_path);
    Json(json!({ "settings": settings }))
}

async fn update_parser_settings(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let normalized = ParserSettings::normalize(payload);
    ensure_parent_dir(&state.parser_settings_path);

    match serde_json::to_string_pretty(&normalized) {
        Ok(serialized) => {
            if let Err(error) = fs::write(&state.parser_settings_path, serialized) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("failed to write parser settings: {}", error) })),
                )
                    .into_response();
            }
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("failed to serialize parser settings: {}", error) })),
            )
                .into_response();
        }
    }

    Json(json!({ "ok": true, "settings": normalized })).into_response()
}

async fn get_parser_events(
    State(state): State<Arc<AppState>>,
    Query(params): Query<EventsParams>,
) -> impl IntoResponse {
    let limit = params.limit.clamp(1, 500);
    let events = tail_parser_events(&state.parser_events_path, limit);
    Json(json!({ "events": events }))
}

async fn get_documents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cache = get_cards_cache_snapshot(&state).await;
    Json(shared_core::get_documents(cache.cards.as_ref(), &state.local_docs_folder))
}

async fn delete_document(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DeleteDocumentRequest>,
) -> impl IntoResponse {
    let target = payload.target.trim().to_ascii_lowercase();

    match shared_core::delete_document(
        &state.local_index_path,
        &state.local_docs_folder,
        &payload.filename,
        &payload.target,
    ) {
        Ok(body) => {
            if target == "index" {
                refresh_index_caches(&state).await;
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(error) if error == "filename is required" || error == "target must be either 'index' or 'folder'" => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response()
        }
        Err(error) if error == "Document not found for selected target" => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "removed_cards": 0,
                "removed_from_folder": false,
                "deleted_path": Value::Null,
                "message": "Document not found for selected target"
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

async fn upload_docx(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut original_filename = String::new();
    let mut parse_immediately = true;

    loop {
        let next = multipart.next_field().await;
        let Some(field) = (match next {
            Ok(value) => value,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("invalid multipart payload: {}", error) })),
                )
                    .into_response();
            }
        }) else {
            break;
        };

        let field_name = field.name().unwrap_or_default().to_string();

        if field_name == "file" {
            original_filename = field.file_name().unwrap_or_default().to_string();
            match field.bytes().await {
                Ok(bytes) => file_bytes = Some(bytes.to_vec()),
                Err(error) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": format!("failed reading uploaded file: {}", error) })),
                    )
                        .into_response();
                }
            }
        } else if field_name == "parse" {
            match field.text().await {
                Ok(value) => parse_immediately = parse_bool_str(&value, true),
                Err(_) => parse_immediately = true,
            }
        }
    }

    if file_bytes.is_none() || original_filename.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "No file uploaded" })),
        )
            .into_response();
    }

    if !original_filename.to_ascii_lowercase().ends_with(".docx") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Only .docx files are supported" })),
        )
            .into_response();
    }

    let safe_name = sanitize_filename(&original_filename);
    let upload_dir = state.local_docs_folder.join("uploaded_docs");
    if let Err(error) = fs::create_dir_all(&upload_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("failed to create upload directory: {}", error) })),
        )
            .into_response();
    }

    let save_path = dedupe_target_path(&upload_dir, &safe_name);
    let stored_filename = save_path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| safe_name.clone());

    if let Err(error) = fs::write(&save_path, file_bytes.unwrap_or_default()) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("failed to write uploaded file: {}", error) })),
        )
            .into_response();
    }

    append_parser_event(
        &state,
        "info",
        &format!("Upload received: {}", stored_filename),
        Some(json!({"source":"api-upload","filename":stored_filename})),
    );

    if !parse_immediately {
        append_parser_event(
            &state,
            "info",
            &format!("Upload stored (deferred parse): {}", stored_filename),
            Some(json!({"source":"api-upload","filename":stored_filename})),
        );

        return Json(json!({
            "ok": true,
            "queued": false,
            "deferred": true,
            "filename": stored_filename,
            "stored_path": save_path.to_string_lossy(),
            "cards_indexed": 0,
            "parse_ms": 0,
        }))
        .into_response();
    }

    let queued = queue_background_parse(state.clone(), save_path.clone(), stored_filename.clone()).await;
    if queued {
        append_parser_event(
            &state,
            "info",
            &format!("Queued parsing: {}", stored_filename),
            Some(json!({"source":"api-upload","filename":stored_filename})),
        );
    }

    Json(json!({
        "ok": true,
        "queued": queued,
        "deferred": false,
        "filename": stored_filename,
        "stored_path": save_path.to_string_lossy(),
        "cards_indexed": 0,
        "parse_ms": 0,
    }))
    .into_response()
}

async fn parse_uploaded_docs(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uploaded = list_uploaded_docx(&state.local_docs_folder);
    let cache = get_cards_cache_snapshot(&state).await;
    let indexed_filenames = &cache.indexed_filenames;

    let mut queued = 0_u64;
    let mut skipped = 0_u64;

    for item in uploaded {
        if indexed_filenames.contains(&item.filename.to_ascii_lowercase()) {
            skipped += 1;
            continue;
        }

        if queue_background_parse(state.clone(), item.absolute_path, item.filename).await {
            queued += 1;
        }
    }

    append_parser_event(
        &state,
        "info",
        &format!("Batch parse queued {} uploaded doc(s), skipped {} already indexed", queued, skipped),
        Some(json!({"source":"api-batch-parse","queued":queued,"skipped":skipped})),
    );

    Json(json!({
        "ok": true,
        "queued": queued,
        "skipped_already_indexed": skipped,
    }))
}

async fn index_document(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<IndexDocumentRequest>,
) -> impl IntoResponse {
    let filename = payload.filename.trim().to_string();
    if filename.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "filename is required" })),
        )
            .into_response();
    }

    let file_item = find_uploaded_docx_file(&state.local_docs_folder, &filename);
    let Some(file_item) = file_item else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Document file not found in uploaded_docs" })),
        )
            .into_response();
    };

    let parse_started = Instant::now();
    let parsed_cards = match parse_docx_cards(&state, &file_item.absolute_path, &file_item.filename).await {
        Ok(cards) => cards,
        Err(error) => {
            append_parser_event(
                &state,
                "error",
                &format!("Failed indexing {}: {}", file_item.filename, error),
                Some(json!({"source":"api-index-document","filename":file_item.filename})),
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Failed to index {}: {}", file_item.filename, error) })),
            )
                .into_response();
        }
    };

    let _ = append_cards_to_index(&state, &parsed_cards, true).await;

    let parse_ms = parse_started.elapsed().as_secs_f64() * 1000.0;
    let cards_per_second = if parse_ms > 0.0 {
        parsed_cards.len() as f64 * 1000.0 / parse_ms
    } else {
        0.0
    };

    append_parser_event(
        &state,
        "info",
        &format!(
            "Indexed {}: {} cards in {:.2}ms ({:.2} cards/s)",
            file_item.filename,
            parsed_cards.len(),
            parse_ms,
            cards_per_second,
        ),
        Some(json!({
            "source":"api-index-document",
            "filename": file_item.filename,
            "cards_indexed": parsed_cards.len(),
            "parse_ms": round2(parse_ms),
            "cards_per_second": round2(cards_per_second),
        })),
    );

    Json(json!({
        "ok": true,
        "filename": file_item.filename,
        "cards_indexed": parsed_cards.len(),
    }))
    .into_response()
}

async fn create_user() -> impl IntoResponse {
    Json(json!({ "ok": true, "noop": true }))
}

fn start_parse_workers(
    state: Arc<AppState>,
    receiver: Arc<Mutex<mpsc::Receiver<ParseJob>>>,
    worker_count: usize,
) {
    for _ in 0..worker_count {
        let state = state.clone();
        let receiver = receiver.clone();
        tokio::spawn(async move {
            loop {
                let maybe_job = {
                    let mut guard = receiver.lock().await;
                    guard.recv().await
                };

                let Some(job) = maybe_job else {
                    break;
                };

                process_parse_job(state.clone(), job).await;
            }
        });
    }
}

async fn process_parse_job(state: Arc<AppState>, job: ParseJob) {
    let parse_started = Instant::now();
    let parse_result = parse_docx_cards(&state, &job.doc_path, &job.stored_filename).await;

    match parse_result {
        Ok(cards) => {
            let _ = append_cards_to_index(&state, &cards, true).await;
            let parse_ms = parse_started.elapsed().as_secs_f64() * 1000.0;
            let cards_per_second = if parse_ms > 0.0 {
                cards.len() as f64 * 1000.0 / parse_ms
            } else {
                0.0
            };

            append_parser_event(
                &state,
                "info",
                &format!(
                    "Parsed {}: {} cards in {:.2}ms ({:.2} cards/s)",
                    job.stored_filename,
                    cards.len(),
                    parse_ms,
                    cards_per_second,
                ),
                Some(json!({
                    "source":"api-upload",
                    "filename": job.stored_filename,
                    "cards_indexed": cards.len(),
                    "parse_ms": round2(parse_ms),
                    "cards_per_second": round2(cards_per_second),
                })),
            );
        }
        Err(error) => {
            append_parser_event(
                &state,
                "error",
                &format!("Failed parsing {}: {}", job.stored_filename, error),
                Some(json!({"source":"api-upload","filename":job.stored_filename})),
            );
        }
    }

    let mut active = state.active_upload_parses.lock().await;
    active.remove(&job.stored_filename);
}

async fn queue_background_parse(state: Arc<AppState>, doc_path: PathBuf, stored_filename: String) -> bool {
    {
        let mut active = state.active_upload_parses.lock().await;
        if active.contains(&stored_filename) {
            return false;
        }
        active.insert(stored_filename.clone());
    }

    if state
        .parse_job_sender
        .send(ParseJob {
            doc_path,
            stored_filename: stored_filename.clone(),
        })
        .await
        .is_ok()
    {
        true
    } else {
        let mut active = state.active_upload_parses.lock().await;
        active.remove(&stored_filename);
        false
    }
}

async fn parse_docx_cards(state: &AppState, doc_path: &Path, filename: &str) -> Result<Vec<Card>, String> {
    let settings = load_parser_settings(&state.parser_settings_path);
    let parser_card_workers = if settings.parser_card_workers == 0 {
        1
    } else {
        settings.parser_card_workers as usize
    };
    let options = ParserBuildOptions {
        use_parallel_processing: settings.use_parallel_processing,
        parser_card_workers,
    };

    parse_docx_via_rust(doc_path, filename, state, &options)
}

async fn run_parser_benchmark(state: Arc<AppState>) {
    let docx_files = discover_docx_files(&state.local_docs_folder);
    if docx_files.is_empty() {
        println!("No .docx files found under {}", state.local_docs_folder.to_string_lossy());
        return;
    }

    let bench_limit = env::var("PARSER_BENCH_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(docx_files.len())
        .max(1);

    let selected = docx_files.into_iter().take(bench_limit).collect::<Vec<_>>();
    let mut rust_total_cards = 0_usize;
    let mut rust_total_ms = 0.0_f64;
    let mut parse_errors = 0_usize;

    for docx_path in &selected {
        let filename = docx_path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown.docx".to_string());

        let rust_started = Instant::now();
        let bench_options = ParserBuildOptions {
            use_parallel_processing: true,
            parser_card_workers: num_cpus::get().max(1),
        };
        let rust_cards = parse_docx_via_rust(docx_path, &filename, &state, &bench_options);
        let rust_ms = rust_started.elapsed().as_secs_f64() * 1000.0;

        let rust_count = rust_cards.as_ref().map(|cards| cards.len()).unwrap_or(0);
        if rust_cards.is_err() {
            parse_errors += 1;
        }
        rust_total_cards += rust_count;
        rust_total_ms += rust_ms;

        println!(
            "BENCH file={} rust_ms={:.2} rust_cards={} rust_ok={}",
            filename,
            rust_ms,
            rust_count,
            rust_cards.is_ok(),
        );
    }

    let rust_cards_per_sec = if rust_total_ms > 0.0 {
        rust_total_cards as f64 * 1000.0 / rust_total_ms
    } else {
        0.0
    };
    println!(
        "BENCH_SUMMARY files={} rust_total_ms={:.2} rust_total_cards={} rust_cards_per_sec={:.2} parse_errors={}",
        selected.len(),
        rust_total_ms,
        rust_total_cards,
        rust_cards_per_sec,
        parse_errors,
    );
}

#[derive(Debug, Clone)]
struct ParsedParagraph {
    style_name: String,
    text: String,
    runs: Vec<ParsedRun>,
}

#[derive(Debug, Clone)]
struct ParsedRun {
    text: String,
    style_name: String,
    is_bold: bool,
    is_underline: bool,
    is_highlighted: bool,
}

#[derive(Debug, Clone)]
struct CardChunkCandidate {
    tag_base: String,
    tag_sub: String,
    cite: String,
    body: Vec<String>,
    cite_date: Option<String>,
    object_id: String,
    highlighted_text: String,
    highlights: Vec<(usize, usize, usize)>,
    underlines: Vec<(usize, usize, usize)>,
    emphasis: Vec<(usize, usize, usize)>,
    cite_emphasis: Vec<(usize, usize)>,
}

#[derive(Debug, Clone)]
struct ParserBuildOptions {
    use_parallel_processing: bool,
    parser_card_workers: usize,
}

fn parse_docx_via_rust(
    doc_path: &Path,
    filename: &str,
    state: &AppState,
    options: &ParserBuildOptions,
) -> Result<Vec<Card>, String> {
    let file = fs::File::open(doc_path).map_err(|error| format!("failed opening docx: {}", error))?;
    let mut archive = ZipArchive::new(file).map_err(|error| format!("failed reading docx zip: {}", error))?;

    let mut document_xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|error| format!("missing word/document.xml: {}", error))?
        .read_to_string(&mut document_xml)
        .map_err(|error| format!("failed reading document xml: {}", error))?;

    let style_map = match archive.by_name("word/styles.xml") {
        Ok(mut styles_file) => {
            let mut styles_xml = String::new();
            if styles_file.read_to_string(&mut styles_xml).is_ok() {
                parse_style_map(&styles_xml)
            } else {
                HashMap::new()
            }
        }
        Err(_) => HashMap::new(),
    };

    let paragraphs = parse_paragraphs_from_document_xml(&document_xml, &style_map)?;
    build_cards_from_paragraphs(&paragraphs, filename, &state.local_docs_folder, options)
}

fn parse_style_map(styles_xml: &str) -> HashMap<String, String> {
    let mut reader = Reader::from_str(styles_xml);
    reader.config_mut().trim_text(true);

    let mut map = HashMap::new();
    let mut current_style_id: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let local_name = String::from_utf8_lossy(event.local_name().as_ref()).to_string();
                if local_name == "style" {
                    for attr in event.attributes().flatten() {
                        if attr.key.local_name().as_ref() == b"styleId" {
                            current_style_id = Some(String::from_utf8_lossy(attr.value.as_ref()).to_string());
                        }
                    }
                } else if local_name == "name" {
                    let mut style_name = String::new();
                    for attr in event.attributes().flatten() {
                        if attr.key.local_name().as_ref() == b"val" {
                            style_name = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                        }
                    }

                    if let Some(style_id) = &current_style_id {
                        if !style_id.is_empty() && !style_name.is_empty() {
                            map.insert(style_id.clone(), style_name);
                        }
                    }
                }
            }
            Ok(Event::End(event)) => {
                if event.local_name().as_ref() == b"style" {
                    current_style_id = None;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    map
}

fn parse_paragraphs_from_document_xml(
    document_xml: &str,
    style_map: &HashMap<String, String>,
) -> Result<Vec<ParsedParagraph>, String> {
    let mut reader = Reader::from_str(document_xml);
    reader.config_mut().trim_text(false);

    let mut paragraphs = Vec::new();
    let mut in_paragraph = false;
    let mut in_text_node = false;
    let mut in_run = false;
    let mut in_run_properties = false;
    let mut paragraph_text = String::new();
    let mut paragraph_style_id = String::new();
    let mut paragraph_runs: Vec<ParsedRun> = Vec::new();

    let mut run_text = String::new();
    let mut run_style_id = String::new();
    let mut run_bold = false;
    let mut run_underline = false;
    let mut run_highlighted = false;

    let flush_run = |paragraph_runs: &mut Vec<ParsedRun>,
                     run_text: &mut String,
                     run_style_id: &str,
                     run_bold: bool,
                     run_underline: bool,
                     run_highlighted: bool| {
        if run_text.is_empty() {
            return;
        }

        let style_name = style_map
            .get(run_style_id)
            .cloned()
            .unwrap_or_else(|| run_style_id.to_string());

        paragraph_runs.push(ParsedRun {
            text: run_text.clone(),
            style_name,
            is_bold: run_bold,
            is_underline: run_underline,
            is_highlighted: run_highlighted,
        });
        run_text.clear();
    };

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let local_name = event.local_name();
                if local_name.as_ref() == b"p" {
                    in_paragraph = true;
                    paragraph_text.clear();
                    paragraph_style_id.clear();
                    paragraph_runs.clear();
                } else if in_paragraph && local_name.as_ref() == b"pStyle" {
                    for attr in event.attributes().flatten() {
                        if attr.key.local_name().as_ref() == b"val" {
                            paragraph_style_id = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                        }
                    }
                } else if in_paragraph && local_name.as_ref() == b"r" {
                    in_run = true;
                    run_text.clear();
                    run_style_id.clear();
                    run_bold = false;
                    run_underline = false;
                    run_highlighted = false;
                } else if in_paragraph && in_run && local_name.as_ref() == b"rPr" {
                    in_run_properties = true;
                } else if in_paragraph && in_run && in_run_properties && local_name.as_ref() == b"rStyle" {
                    for attr in event.attributes().flatten() {
                        if attr.key.local_name().as_ref() == b"val" {
                            run_style_id = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                        }
                    }
                } else if in_paragraph && in_run && in_run_properties && local_name.as_ref() == b"b" {
                    run_bold = true;
                } else if in_paragraph && in_run && in_run_properties && local_name.as_ref() == b"u" {
                    run_underline = true;
                } else if in_paragraph && in_run && in_run_properties && local_name.as_ref() == b"highlight" {
                    run_highlighted = true;
                } else if in_paragraph && local_name.as_ref() == b"t" {
                    in_text_node = true;
                } else if in_paragraph && local_name.as_ref() == b"tab" {
                    paragraph_text.push('\t');
                    if in_run {
                        run_text.push('\t');
                    }
                }
            }
            Ok(Event::Empty(event)) => {
                let local_name = event.local_name();
                if in_paragraph && local_name.as_ref() == b"pStyle" {
                    for attr in event.attributes().flatten() {
                        if attr.key.local_name().as_ref() == b"val" {
                            paragraph_style_id = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                        }
                    }
                } else if in_paragraph && in_run && in_run_properties && local_name.as_ref() == b"rStyle" {
                    for attr in event.attributes().flatten() {
                        if attr.key.local_name().as_ref() == b"val" {
                            run_style_id = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                        }
                    }
                } else if in_paragraph && in_run && in_run_properties && local_name.as_ref() == b"b" {
                    run_bold = true;
                } else if in_paragraph && in_run && in_run_properties && local_name.as_ref() == b"u" {
                    run_underline = true;
                } else if in_paragraph && in_run && in_run_properties && local_name.as_ref() == b"highlight" {
                    run_highlighted = true;
                } else if in_paragraph && local_name.as_ref() == b"tab" {
                    paragraph_text.push('\t');
                    if in_run {
                        run_text.push('\t');
                    }
                }
            }
            Ok(Event::Text(text)) => {
                if in_paragraph && in_text_node {
                    let value = text.decode().map_err(|error| format!("xml decode error: {}", error))?;
                    paragraph_text.push_str(&value);
                    if in_run {
                        run_text.push_str(&value);
                    }
                }
            }
            Ok(Event::End(event)) => {
                let local_name = event.local_name();
                if local_name.as_ref() == b"t" {
                    in_text_node = false;
                } else if local_name.as_ref() == b"rPr" {
                    in_run_properties = false;
                } else if local_name.as_ref() == b"r" {
                    flush_run(
                        &mut paragraph_runs,
                        &mut run_text,
                        &run_style_id,
                        run_bold,
                        run_underline,
                        run_highlighted,
                    );
                    in_run = false;
                    in_run_properties = false;
                } else if local_name.as_ref() == b"p" {
                    let mut style_name = style_map
                        .get(&paragraph_style_id)
                        .cloned()
                        .unwrap_or_else(|| paragraph_style_id.clone());

                    if style_name.trim().is_empty() {
                        style_name = "Normal".to_string();
                    }

                    paragraphs.push(ParsedParagraph {
                        style_name,
                        text: paragraph_text.trim_end().to_string(),
                        runs: paragraph_runs.clone(),
                    });

                    in_paragraph = false;
                    in_text_node = false;
                    in_run = false;
                    in_run_properties = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(format!("xml parse error: {}", error)),
            _ => {}
        }
    }

    Ok(paragraphs)
}

fn build_cards_from_paragraphs(
    paragraphs: &[ParsedParagraph],
    filename: &str,
    local_docs_folder: &Path,
    options: &ParserBuildOptions,
) -> Result<Vec<Card>, String> {
    static TAG_STYLE_KEYS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
        HashSet::from(["heading4", "tags", "tag", "heading3", "heading2"])
    });
    static BODY_STYLE_KEYS: Lazy<HashSet<&'static str>> =
        Lazy::new(|| HashSet::from(["normal", "cards", "card", "normalweb", "normalcard"]));
    static LIST_PARAGRAPH_STYLE_KEYS: Lazy<HashSet<&'static str>> =
        Lazy::new(|| HashSet::from(["listparagraph", "listbullet", "listnumber"]));
    static DIGIT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d").expect("valid regex"));

    let mut card_chunks: Vec<Vec<ParsedParagraph>> = Vec::new();
    let mut current: Vec<ParsedParagraph> = Vec::new();
    let mut current_has_only_tags = true;

    let trace_doc_filter = env::var("PARSER_TRACE_DOC").ok();
    let trace_enabled = trace_doc_filter
        .as_ref()
        .map(|value| filename.to_ascii_lowercase().contains(&value.to_ascii_lowercase()))
        .unwrap_or(false);

    for paragraph in paragraphs {
        let style_key = normalize_style_key(&paragraph.style_name);
        if style_matches_key_set(&style_key, &TAG_STYLE_KEYS) {
            if !current.is_empty() && current_has_only_tags {
                current.push(paragraph.clone());
                continue;
            }

            if !current.is_empty() {
                card_chunks.push(current);
            }

            current = vec![paragraph.clone()];
            current_has_only_tags = true;
        } else {
            current.push(paragraph.clone());
            current_has_only_tags = false;
        }
    }

    if !current.is_empty() {
        card_chunks.push(current);
    }

    let chunk_count = card_chunks.len();

    let use_parallel_build = options.use_parallel_processing
        && options.parser_card_workers > 1
        && chunk_count >= 64
        && !trace_enabled;

    if trace_enabled {
        info!(
            "PARSER_TRACE_START file={} paragraphs={} chunks={} workers={}",
            filename,
            paragraphs.len(),
            chunk_count,
            options.parser_card_workers
        );
    }

    let mut parsed = if use_parallel_build {
        if let Ok(pool) = rayon::ThreadPoolBuilder::new()
            .num_threads(options.parser_card_workers)
            .build()
        {
            pool.install(|| {
                card_chunks
                    .into_par_iter()
                    .enumerate()
                    .map(|(index, chunk)| {
                        (
                            index,
                            parse_chunk_candidate(
                                &chunk,
                                filename,
                                &TAG_STYLE_KEYS,
                                &BODY_STYLE_KEYS,
                                &LIST_PARAGRAPH_STYLE_KEYS,
                                &DIGIT_RE,
                            ),
                        )
                    })
                    .collect::<Vec<_>>()
            })
        } else {
            card_chunks
                .iter()
                .enumerate()
                .map(|(index, chunk)| {
                    (
                        index,
                        parse_chunk_candidate(
                            chunk,
                            filename,
                            &TAG_STYLE_KEYS,
                            &BODY_STYLE_KEYS,
                            &LIST_PARAGRAPH_STYLE_KEYS,
                            &DIGIT_RE,
                        ),
                    )
                })
                .collect::<Vec<_>>()
        }
    } else {
        card_chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| {
                if trace_enabled && index < 40 {
                    let first_style = chunk
                        .first()
                        .map(|paragraph| paragraph.style_name.as_str())
                        .unwrap_or("");
                    let first_text = chunk
                        .first()
                        .map(|paragraph| paragraph.text.replace('\n', " "))
                        .unwrap_or_default();
                    info!(
                        "PARSER_TRACE_CHUNK file={} chunk={} size={} first_style={} first_text={}",
                        filename,
                        index,
                        chunk.len(),
                        first_style,
                        first_text.chars().take(140).collect::<String>()
                    );
                }

                let candidate = if trace_enabled {
                    match parse_chunk_candidate_detailed(
                        chunk,
                        filename,
                        &TAG_STYLE_KEYS,
                        &BODY_STYLE_KEYS,
                        &LIST_PARAGRAPH_STYLE_KEYS,
                        &DIGIT_RE,
                    ) {
                        Ok(item) => Some(item),
                        Err(reason) => {
                            if trace_enabled && index < 40 {
                                info!(
                                    "PARSER_TRACE_REASON file={} chunk={} reason={}",
                                    filename,
                                    index,
                                    reason
                                );
                            }
                            None
                        }
                    }
                } else {
                    parse_chunk_candidate(
                        chunk,
                        filename,
                        &TAG_STYLE_KEYS,
                        &BODY_STYLE_KEYS,
                        &LIST_PARAGRAPH_STYLE_KEYS,
                        &DIGIT_RE,
                    )
                };

                (index, candidate)
            })
            .collect::<Vec<_>>()
    };

    parsed.sort_by_key(|(index, _)| *index);

    let mut output = Vec::new();
    let mut dropped = 0usize;
    for (index, candidate) in parsed {
        if let Some(item) = candidate {
            output.push(item);
        } else {
            dropped += 1;
            if trace_enabled && index < 40 {
                info!("PARSER_TRACE_DROP file={} chunk={}", filename, index);
            }
        }
    }

    if trace_enabled {
        info!(
            "PARSER_TRACE_SUMMARY file={} chunks={} cards={} dropped={}",
            filename,
            chunk_count,
            output.len(),
            dropped
        );
    }

    let registry_path = env::var("CARD_ID_REGISTRY_PATH")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| local_docs_folder.join("card_id_registry.json"));
    let keys = output
        .iter()
        .map(|candidate| card_registry_key(filename, &candidate.tag_base, &candidate.cite, &candidate.body))
        .collect::<Vec<_>>();
    let numbers_by_key = assign_registry_numbers(&registry_path, &keys)?;

    let mut cards = Vec::with_capacity(output.len());
    for candidate in output {
        let key = card_registry_key(filename, &candidate.tag_base, &candidate.cite, &candidate.body);
        let Some(card_number) = numbers_by_key.get(&key).copied() else {
            continue;
        };
        let card_identifier = format_card_identifier(card_number);
        let card_identifier_token = format!("[[{}]]", card_identifier);
        let tag = format!("{} {}", candidate.tag_base, card_identifier_token)
            .trim()
            .to_string();

        let mut card = Card::new();
        card.insert("tag".to_string(), Value::String(tag));
        card.insert("tag_base".to_string(), Value::String(candidate.tag_base));
        card.insert("card_number".to_string(), Value::from(card_number));
        card.insert("card_identifier".to_string(), Value::String(card_identifier.clone()));
        card.insert("card_identifier_token".to_string(), Value::String(card_identifier_token));
        card.insert("tag_sub".to_string(), Value::String(candidate.tag_sub));
        card.insert("cite".to_string(), Value::String(candidate.cite));
        card.insert(
            "body".to_string(),
            Value::Array(candidate.body.into_iter().map(Value::String).collect()),
        );
        card.insert("id".to_string(), Value::String(candidate.object_id));
        card.insert("highlighted_text".to_string(), Value::String(candidate.highlighted_text));
        card.insert(
            "highlights".to_string(),
            Value::Array(
                candidate
                    .highlights
                    .into_iter()
                    .map(|(a, b, c)| Value::Array(vec![Value::from(a), Value::from(b), Value::from(c)]))
                    .collect(),
            ),
        );
        card.insert(
            "underlines".to_string(),
            Value::Array(
                candidate
                    .underlines
                    .into_iter()
                    .map(|(a, b, c)| Value::Array(vec![Value::from(a), Value::from(b), Value::from(c)]))
                    .collect(),
            ),
        );
        card.insert(
            "emphasis".to_string(),
            Value::Array(
                candidate
                    .emphasis
                    .into_iter()
                    .map(|(a, b, c)| Value::Array(vec![Value::from(a), Value::from(b), Value::from(c)]))
                    .collect(),
            ),
        );
        card.insert(
            "cite_emphasis".to_string(),
            Value::Array(
                candidate
                    .cite_emphasis
                    .into_iter()
                    .map(|(a, b)| Value::Array(vec![Value::from(a), Value::from(b)]))
                    .collect(),
            ),
        );
        card.insert("filename".to_string(), Value::String(filename.to_string()));
        card.insert("division".to_string(), Value::String("local".to_string()));
        card.insert("year".to_string(), Value::String("local".to_string()));
        card.insert("school".to_string(), Value::String("Local".to_string()));
        card.insert("team".to_string(), Value::String("Local".to_string()));
        card.insert("download_url".to_string(), Value::String("local".to_string()));

        if let Some(date) = candidate.cite_date {
            card.insert("cite_date".to_string(), Value::String(date));
        }

        cards.push(card);
    }

    Ok(cards)
}

fn parse_chunk_candidate(
    chunk: &[ParsedParagraph],
    filename: &str,
    tag_style_keys: &HashSet<&'static str>,
    body_style_keys: &HashSet<&'static str>,
    list_style_keys: &HashSet<&'static str>,
    digit_re: &Regex,
) -> Option<CardChunkCandidate> {
    parse_chunk_candidate_detailed(
        chunk,
        filename,
        tag_style_keys,
        body_style_keys,
        list_style_keys,
        digit_re,
    )
    .ok()
}

fn parse_chunk_candidate_detailed(
    chunk: &[ParsedParagraph],
    filename: &str,
    tag_style_keys: &HashSet<&'static str>,
    body_style_keys: &HashSet<&'static str>,
    list_style_keys: &HashSet<&'static str>,
    digit_re: &Regex,
) -> Result<CardChunkCandidate, String> {
    if chunk.len() < 2 {
        return Err("chunk_too_short".to_string());
    }
    if !style_matches_key_set(&normalize_style_key(&chunk[0].style_name), tag_style_keys) {
        return Err("first_style_not_tag".to_string());
    }

    let mut cite_index: Option<usize> = None;
    let mut pre_cite_non_empty: Vec<(usize, String, String)> = Vec::new();
    for (index, paragraph) in chunk.iter().enumerate().skip(1) {
        if digit_re.is_match(&paragraph.text) {
            cite_index = Some(index);
            break;
        }
        let text = paragraph.text.trim().to_string();
        if !text.is_empty() {
            pre_cite_non_empty.push((
                index,
                text,
                normalize_style_key(&paragraph.style_name),
            ));
        }
    }

    let Some(cite_i) = cite_index else {
        return Err("missing_cite_digit".to_string());
    };

    let mut tag_base = chunk[0].text.trim().trim_matches(',').trim().to_string();
    let mut consumed_tag_source_index: Option<usize> = None;
    if tag_base.is_empty() {
        if let Some((source_index, text, _)) = pre_cite_non_empty
            .iter()
            .find(|(_, _, style_key)| style_matches_key_set(style_key, tag_style_keys))
        {
            tag_base = text.clone();
            consumed_tag_source_index = Some(*source_index);
        } else if let Some((source_index, text, _)) = pre_cite_non_empty.first() {
            tag_base = text.clone();
            consumed_tag_source_index = Some(*source_index);
        }
    }

    let tag_sub_lines = pre_cite_non_empty
        .into_iter()
        .filter(|(source_index, _, _)| Some(*source_index) != consumed_tag_source_index)
        .map(|(_, text, _)| text)
        .collect::<Vec<_>>();

    let cite_paragraph = &chunk[cite_i];
    let cite = cite_paragraph.text.trim().to_string();
    if cite.is_empty() {
        return Err("empty_cite".to_string());
    }

    let body = chunk
        .iter()
        .skip(cite_i + 1)
        .filter(|paragraph| {
            let style_key = normalize_style_key(&paragraph.style_name);
            style_matches_key_set(&style_key, body_style_keys)
                || style_matches_key_set(&style_key, list_style_keys)
        })
        .map(|paragraph| paragraph.text.clone())
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();

    if body.join("").chars().count() < 25 {
        let strict_chars = body.join("").chars().count();
        let post_cite_total = chunk.iter().skip(cite_i + 1).count();
        let post_cite_non_empty = chunk
            .iter()
            .skip(cite_i + 1)
            .filter(|paragraph| !paragraph.text.trim().is_empty())
            .count();
        let style_preview = chunk
            .iter()
            .skip(cite_i + 1)
            .map(|paragraph| normalize_style_key(&paragraph.style_name))
            .take(8)
            .collect::<Vec<_>>()
            .join(",");
        return Err(format!(
            "body_too_short strict_chars={} strict_paras={} post_total={} post_non_empty={} cite_i={} post_styles={}",
            strict_chars,
            body.len(),
            post_cite_total,
            post_cite_non_empty,
            cite_i,
            style_preview
        ));
    }

    let mut cite_emphasis: Vec<(usize, usize)> = Vec::new();
    let mut cite_cursor = 0_usize;
    let cite_text = cite_paragraph.text.as_str();
    for run in &cite_paragraph.runs {
        let mut run_text = run.text.clone();
        if run_text.trim().is_empty() {
            continue;
        }

        let mut run_index = find_run_index(cite_text, &run_text, cite_cursor);
        if run_index.is_none() {
            let stripped = run_text.trim().to_string();
            if !stripped.is_empty() {
                run_index = find_run_index(cite_text, &stripped, cite_cursor);
                if run_index.is_some() {
                    run_text = stripped;
                }
            }
        }

        let Some(start) = run_index else {
            continue;
        };
        let end = start + run_text.len();
        let run_style_key = normalize_style_key(&run.style_name);
        if run_style_key == "13ptbold" || run_style_key == "cites" || run.is_bold {
            cite_emphasis.push((start, end));
        }
        cite_cursor = end;
    }

    let mut highlights: Vec<(usize, usize, usize)> = Vec::new();
    let mut underlines: Vec<(usize, usize, usize)> = Vec::new();
    let mut emphasis: Vec<(usize, usize, usize)> = Vec::new();
    let mut highlighted_parts: Vec<String> = Vec::new();

    let mut p_index = 2_usize;
    for paragraph in chunk.iter().skip(cite_i + 1) {
        let style_key = normalize_style_key(&paragraph.style_name);
        let is_body_paragraph = style_matches_key_set(&style_key, body_style_keys)
            || style_matches_key_set(&style_key, list_style_keys);
        if !is_body_paragraph {
            p_index += 1;
            continue;
        }

        let mut paragraph_cursor = 0_usize;
        for run in &paragraph.runs {
            let mut run_text = run.text.clone();
            if run_text.trim().is_empty() {
                continue;
            }

            let mut run_index = find_run_index(&paragraph.text, &run_text, paragraph_cursor);
            if run_index.is_none() {
                let stripped = run_text.trim().to_string();
                if !stripped.is_empty() {
                    run_index = find_run_index(&paragraph.text, &stripped, paragraph_cursor);
                    if run_index.is_some() {
                        run_text = stripped;
                    }
                }
            }

            let Some(start) = run_index else {
                continue;
            };
            let end = start + run_text.len();

            if run.is_highlighted {
                highlights.push((p_index, start, end));
                highlighted_parts.push(run_text.clone());
            }

            let run_style_key = normalize_style_key(&run.style_name);
            if run_style_key.contains("underline") || run.is_underline {
                underlines.push((p_index, start, end));
            }
            if run_style_key.contains("emphasis") {
                emphasis.push((p_index, start, end));
            }

            paragraph_cursor = end;
        }

        p_index += 1;
    }

    let mut hasher = Sha256::new();
    hasher.update(format!("{}\n{}\n{}\n{}\n", filename, tag_base, cite, body.join("\n")));
    let object_id = format!("{:x}", hasher.finalize());

    Ok(CardChunkCandidate {
        tag_base,
        tag_sub: if tag_sub_lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", tag_sub_lines.join("\n"))
        },
        cite_date: infer_cite_date(&cite),
        cite,
        body,
        object_id,
        highlighted_text: highlighted_parts.join(" "),
        highlights,
        underlines,
        emphasis,
        cite_emphasis,
    })
}

fn normalize_style_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect::<String>()
}

fn style_matches_key_set(style_key: &str, set: &HashSet<&'static str>) -> bool {
    if set.contains(style_key) {
        return true;
    }

    let trimmed = style_key.trim_end_matches(|ch: char| ch.is_ascii_digit());
    if trimmed.is_empty() {
        return false;
    }

    set.contains(trimmed)
}

fn format_card_identifier(card_number: u64) -> String {
    let padded = format!("{:010}", card_number);
    format!("CID-{}-{}", &padded[0..5], &padded[5..10])
}

fn card_registry_key(filename: &str, tag_base: &str, cite: &str, body: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(
        format!(
            "{}\n{}\n{}\n{}",
            filename.trim().to_ascii_lowercase(),
            tag_base,
            cite,
            body.join("\n")
        )
        .as_bytes(),
    );
    format!("{:x}", hasher.finalize())
}

fn find_run_index(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    if needle.is_empty() || from > haystack.len() {
        return None;
    }

    haystack
        .get(from..)
        .and_then(|slice| slice.find(needle).map(|index| from + index))
}

#[derive(Debug, Serialize, Deserialize)]
struct CardIdRegistryData {
    next_number: u64,
    used_numbers: Vec<u64>,
    assigned_by_key: HashMap<String, u64>,
}

impl Default for CardIdRegistryData {
    fn default() -> Self {
        Self {
            next_number: 1,
            used_numbers: Vec::new(),
            assigned_by_key: HashMap::new(),
        }
    }
}

fn read_registry_unlocked(handle: &mut fs::File) -> CardIdRegistryData {
    let mut raw = String::new();
    if handle.seek(SeekFrom::Start(0)).is_err() || handle.read_to_string(&mut raw).is_err() {
        return CardIdRegistryData::default();
    }

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return CardIdRegistryData::default();
    }

    let parsed = serde_json::from_str::<CardIdRegistryData>(trimmed).unwrap_or_default();

    let next_number = if parsed.next_number == 0 { 1 } else { parsed.next_number };
    let used_numbers = parsed
        .used_numbers
        .into_iter()
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    let assigned_by_key = parsed
        .assigned_by_key
        .into_iter()
        .filter(|(key, value)| !key.trim().is_empty() && *value > 0)
        .collect::<HashMap<_, _>>();

    CardIdRegistryData {
        next_number,
        used_numbers,
        assigned_by_key,
    }
}

fn write_registry_unlocked(handle: &mut fs::File, data: &CardIdRegistryData) -> Result<(), String> {
    handle
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("registry seek failed: {}", error))?;
    handle
        .set_len(0)
        .map_err(|error| format!("registry truncate failed: {}", error))?;

    let serialized = serde_json::to_string_pretty(data)
        .map_err(|error| format!("registry encode failed: {}", error))?;
    handle
        .write_all(serialized.as_bytes())
        .and_then(|_| handle.flush())
        .map_err(|error| format!("registry write failed: {}", error))
}

fn assign_registry_numbers(registry_path: &Path, card_keys: &[String]) -> Result<HashMap<String, u64>, String> {
    let keys = card_keys
        .iter()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect::<Vec<_>>();

    if keys.is_empty() {
        return Ok(HashMap::new());
    }

    ensure_parent_dir(registry_path);
    let mut handle = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(registry_path)
        .map_err(|error| format!("registry open failed: {}", error))?;

    let lock_result = unsafe { libc::flock(handle.as_raw_fd(), libc::LOCK_EX) };
    if lock_result != 0 {
        return Err("registry lock failed".to_string());
    }

    let mut registry = read_registry_unlocked(&mut handle);
    let mut numbers_by_key = HashMap::new();
    let mut seen = HashSet::new();

    for key in keys {
        if !seen.insert(key.clone()) {
            if let Some(value) = registry.assigned_by_key.get(&key).copied() {
                numbers_by_key.insert(key, value);
            }
            continue;
        }

        if let Some(existing) = registry.assigned_by_key.get(&key).copied() {
            numbers_by_key.insert(key, existing);
            continue;
        }

        let next_number = registry.next_number.max(1);
        registry.assigned_by_key.insert(key.clone(), next_number);
        registry.used_numbers.push(next_number);
        numbers_by_key.insert(key, next_number);
        registry.next_number = next_number + 1;
    }

    let write_result = write_registry_unlocked(&mut handle, &registry);
    let _ = unsafe { libc::flock(handle.as_raw_fd(), libc::LOCK_UN) };
    write_result?;

    Ok(numbers_by_key)
}

fn append_to_year_string(year: &str) -> String {
    match year.parse::<u32>() {
        Ok(value) if value <= 21 => format!("20{:02}", value),
        Ok(value) => format!("19{}", value),
        Err(_) => year.to_string(),
    }
}

fn infer_cite_date(cite: &str) -> Option<String> {
    static WORD_STRIP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^a-zA-Z0-9/-]").expect("valid regex"));

    let words = cite
        .split_whitespace()
        .map(|word| WORD_STRIP_RE.replace_all(word, "").to_string())
        .filter(|word| {
            let lowered = word.to_ascii_lowercase();
            lowered != "and" && lowered != "or" && lowered != "of"
        })
        .take_while(|word| word.to_ascii_lowercase() != "accessed")
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();

    if words.is_empty() {
        return None;
    }

    let mut best: Option<(i32, u32, u32, i32)> = None;
    let mut captured_year: Option<String> = None;

    for start in 0..words.len() {
        let end_max = (start + 4).min(words.len());
        for end in (start + 1)..=end_max {
            let mut combo = words[start..end].to_vec();
            let mut weight: i32 = combo.iter().map(|word| word.len() as i32).sum();

            if combo.len() == 1 && combo[0].chars().all(|ch| ch.is_ascii_digit()) && combo[0].len() <= 2 {
                weight -= combo[0].len() as i32;
                combo[0] = append_to_year_string(&combo[0]);
                weight += combo[0].len() as i32;
                if captured_year.is_none() {
                    captured_year = Some(combo[0].clone());
                }
            }

            if combo.iter().all(|token| !token.chars().all(|ch| ch.is_ascii_digit()) || token.len() < 4) {
                if let Some(year) = &captured_year {
                    combo.push(year.clone());
                    weight += year.len() as i32;
                }
            }

            if let Some((year, month, day)) = parse_combo_date(&combo.join(" ")) {
                if year > 1900 {
                    match &best {
                        Some((_, _, _, best_weight)) if *best_weight >= weight => {}
                        _ => best = Some((year, month, day, weight)),
                    }
                }
            }
        }
    }

    best.map(|(year, month, day, _)| format!("{:04}-{:02}-{:02}", year, month, day))
}

fn parse_combo_date(input: &str) -> Option<(i32, u32, u32)> {
    static YEAR_TOKEN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{4}\b").expect("valid regex"));

    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    for format in [
        "%m-%d-%Y",
        "%m/%d/%Y",
        "%Y-%m-%d",
        "%Y/%m/%d",
        "%B %d %Y",
        "%b %d %Y",
        "%d %B %Y",
        "%d %b %Y",
        "%B %Y",
        "%b %Y",
        "%Y",
    ] {
        if let Ok(date) = NaiveDate::parse_from_str(trimmed, format) {
            return Some((date.year(), date.month(), date.day()));
        }
    }

    if YEAR_TOKEN_RE.is_match(trimmed) {
        let cleaned = trimmed.replace(',', " ").replace('.', " ");
        let tokenized = cleaned
            .split_whitespace()
            .map(|token| token.to_string())
            .collect::<Vec<_>>();

        if tokenized.len() == 2 {
            if let Ok(year) = tokenized[1].parse::<i32>() {
                if let Some(month) = month_from_token(&tokenized[0]) {
                    return Some((year, month, 1));
                }
            }
        }
    }

    None
}

fn month_from_token(value: &str) -> Option<u32> {
    match value.to_ascii_lowercase().as_str() {
        "january" | "jan" => Some(1),
        "february" | "feb" => Some(2),
        "march" | "mar" => Some(3),
        "april" | "apr" => Some(4),
        "may" => Some(5),
        "june" | "jun" => Some(6),
        "july" | "jul" => Some(7),
        "august" | "aug" => Some(8),
        "september" | "sep" | "sept" => Some(9),
        "october" | "oct" => Some(10),
        "november" | "nov" => Some(11),
        "december" | "dec" => Some(12),
        _ => None,
    }
}

async fn append_cards_to_index(
    state: &AppState,
    new_cards: &[Card],
    force_upload: bool,
) -> Result<usize, String> {
    if new_cards.is_empty() {
        return Ok(0);
    }

    let _guard = state.index_lock.lock().await;
    let mut existing_cards = load_cards(&state.local_index_path);

    let normalized_filename = string_field(&new_cards[0], "filename").trim().to_ascii_lowercase();
    if !force_upload
        && !normalized_filename.is_empty()
        && existing_cards.iter().any(|card| string_field(card, "filename").trim().to_ascii_lowercase() == normalized_filename)
    {
        return Ok(0);
    }

    let mut seen_ids: HashSet<String> = existing_cards
        .iter()
        .map(|card| string_field(card, "id"))
        .filter(|id| !id.is_empty())
        .collect();

    let mut to_append = Vec::new();
    for card in new_cards {
        let card_id = string_field(card, "id");
        if card_id.is_empty() || seen_ids.contains(&card_id) {
            continue;
        }
        seen_ids.insert(card_id);
        to_append.push(card.clone());
    }

    existing_cards.extend(to_append.clone());
    write_cards_jsonl(&state.local_index_path, &existing_cards)
        .map_err(|error| format!("failed writing index: {}", error))?;

    refresh_index_caches(state).await;

    Ok(to_append.len())
}

fn index_signature(path: &Path) -> IndexSignature {
    let metadata = match fs::metadata(path) {
        Ok(value) => value,
        Err(_) => return IndexSignature::empty(),
    };

    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    IndexSignature {
        size: metadata.len(),
        modified_ns,
    }
}

fn build_cards_cache(signature: IndexSignature, cards: Vec<Card>) -> CardsCache {
    let cards = Arc::new(cards);
    let mut query_cards: Vec<QueryCardCache> = Vec::with_capacity(cards.len());
    let mut term_index: HashMap<String, RoaringBitmap> = HashMap::new();
    let mut id_lookup: HashMap<String, usize> = HashMap::new();
    let mut filename_counts: HashMap<String, (String, u64)> = HashMap::new();
    let mut schools = BTreeSet::new();
    let mut indexed_filenames = HashSet::new();

    for (index, card) in cards.iter().enumerate() {
        let query_card = build_query_card_cache(card);
        for token in extract_index_terms(&query_card.search_blob_lc) {
            if let Ok(index_u32) = u32::try_from(index) {
                term_index.entry(token).or_default().insert(index_u32);
            }
        }
        query_cards.push(query_card);

        let card_id = string_field(card, "id");
        if !card_id.is_empty() {
            id_lookup.insert(card_id, index);
        }

        let filename = string_field(card, "filename").trim().to_string();
        if !filename.is_empty() {
            let key = filename.to_ascii_lowercase();
            let entry = filename_counts.entry(key.clone()).or_insert((filename.clone(), 0));
            entry.1 += 1;
            indexed_filenames.insert(key);
        }

        let school = string_field(card, "school").trim().to_string();
        if !school.is_empty() {
            schools.insert(school);
        }
    }

    CardsCache {
        signature,
        cards,
        query_cards: Arc::new(query_cards),
        term_index: Arc::new(term_index),
        id_lookup: Arc::new(id_lookup),
        filename_counts: Arc::new(filename_counts),
        schools: Arc::new(schools),
        indexed_filenames: Arc::new(indexed_filenames),
    }
}

fn build_query_card_cache(card: &Card) -> QueryCardCache {
    QueryCardCache {
        filename_lc: string_field(card, "filename").to_ascii_lowercase(),
        division_lc: string_field(card, "division").to_ascii_lowercase(),
        year_lc: string_field(card, "year").to_ascii_lowercase(),
        school_lc: string_field(card, "school").to_ascii_lowercase(),
        cite_lc: string_field(card, "cite").to_ascii_lowercase(),
        search_blob_lc: card_search_blob(card),
        cite_ts: card_timestamp(card),
    }
}

fn snapshot_from_cache(cache: &CardsCache) -> CardsCacheSnapshot {
    CardsCacheSnapshot {
        signature: cache.signature.clone(),
        cards: cache.cards.clone(),
        query_cards: cache.query_cards.clone(),
        term_index: cache.term_index.clone(),
        id_lookup: cache.id_lookup.clone(),
        filename_counts: cache.filename_counts.clone(),
        schools: cache.schools.clone(),
        indexed_filenames: cache.indexed_filenames.clone(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedCardsCache {
    signature: IndexSignature,
    cards: Vec<Card>,
    query_cards: Vec<QueryCardCache>,
    term_index: HashMap<String, RoaringBitmap>,
    id_lookup: HashMap<String, usize>,
    filename_counts: HashMap<String, (String, u64)>,
    schools: BTreeSet<String>,
    indexed_filenames: HashSet<String>,
}

fn persisted_from_cache(cache: &CardsCache) -> PersistedCardsCache {
    PersistedCardsCache {
        signature: cache.signature.clone(),
        cards: cache.cards.as_ref().clone(),
        query_cards: cache.query_cards.as_ref().clone(),
        term_index: cache.term_index.as_ref().clone(),
        id_lookup: cache.id_lookup.as_ref().clone(),
        filename_counts: cache.filename_counts.as_ref().clone(),
        schools: cache.schools.as_ref().clone(),
        indexed_filenames: cache.indexed_filenames.as_ref().clone(),
    }
}

fn cache_from_persisted(persisted: PersistedCardsCache) -> CardsCache {
    CardsCache {
        signature: persisted.signature,
        cards: Arc::new(persisted.cards),
        query_cards: Arc::new(persisted.query_cards),
        term_index: Arc::new(persisted.term_index),
        id_lookup: Arc::new(persisted.id_lookup),
        filename_counts: Arc::new(persisted.filename_counts),
        schools: Arc::new(persisted.schools),
        indexed_filenames: Arc::new(persisted.indexed_filenames),
    }
}

fn load_cards_cache_from_disk(cache_path: &Path, expected_signature: &IndexSignature) -> Option<CardsCache> {
    let bytes = fs::read(cache_path).ok()?;
    let persisted: PersistedCardsCache = bincode::deserialize(&bytes).ok()?;
    if &persisted.signature != expected_signature {
        return None;
    }
    Some(cache_from_persisted(persisted))
}

fn save_cards_cache_to_disk(cache_path: &Path, cache: &CardsCache) {
    ensure_parent_dir(cache_path);
    let persisted = persisted_from_cache(cache);
    if let Ok(bytes) = bincode::serialize(&persisted) {
        let _ = fs::write(cache_path, bytes);
    }
}

async fn get_cards_cache_snapshot(state: &AppState) -> CardsCacheSnapshot {
    let signature = index_signature(&state.local_index_path);

    {
        let cache = state.cards_cache.lock().await;
        if cache.signature == signature {
            return snapshot_from_cache(&cache);
        }
    }

    let rebuilt = load_cards_cache_from_disk(&state.index_cache_path, &signature)
        .unwrap_or_else(|| {
            let built = build_cards_cache(signature.clone(), load_cards(&state.local_index_path));
            save_cards_cache_to_disk(&state.index_cache_path, &built);
            built
        });

    let mut cache = state.cards_cache.lock().await;
    if cache.signature != signature {
        *cache = rebuilt;
    }

    snapshot_from_cache(&cache)
}

async fn refresh_index_caches(state: &AppState) {
    let signature = index_signature(&state.local_index_path);
    let rebuilt = build_cards_cache(signature, load_cards(&state.local_index_path));

    {
        let mut cache = state.cards_cache.lock().await;
        *cache = rebuilt;
        save_cards_cache_to_disk(&state.index_cache_path, &cache);
    }

    for shard in state.query_state_shards.iter() {
        shard.lock().await.clear();
    }
    for shard in state.query_result_shards.iter() {
        shard.lock().await.clear();
    }
}

fn query_state_key(params: &QueryParams, sort_by: &str, cite_match: &str, match_mode: &str) -> String {
    [
        params.search.as_str(),
        params.start_date.as_str(),
        params.end_date.as_str(),
        params.exclude_sides.as_str(),
        params.exclude_division.as_str(),
        params.exclude_schools.as_str(),
        params.exclude_years.as_str(),
        sort_by,
        cite_match,
        match_mode,
    ]
    .join("\u{241f}")
}

fn is_indexable_term(token: &str) -> bool {
    let trimmed = token.trim();
    if trimmed.len() < 4 {
        return false;
    }

    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if !first.is_ascii_alphabetic() {
        return false;
    }

    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn extract_index_terms(text: &str) -> Vec<String> {
    let mut start: Option<usize> = None;
    let mut first_is_alpha = false;
    let mut seen: HashSet<String> = HashSet::new();

    let push_token = |end: usize, start_idx: usize, first_alpha: bool, seen_set: &mut HashSet<String>| {
        if !first_alpha {
            return;
        }
        let token = &text[start_idx..end];
        if is_indexable_term(token) {
            seen_set.insert(token.to_string());
        }
    };

    for (idx, ch) in text.char_indices() {
        let is_token_char = ch.is_ascii_alphanumeric() || ch == '_' || ch == '-';
        if is_token_char {
            if start.is_none() {
                start = Some(idx);
                first_is_alpha = ch.is_ascii_alphabetic();
            }
            continue;
        }

        if let Some(start_idx) = start {
            push_token(idx, start_idx, first_is_alpha, &mut seen);
        }
        start = None;
        first_is_alpha = false;
    }

    if let Some(start_idx) = start {
        push_token(text.len(), start_idx, first_is_alpha, &mut seen);
    }

    seen.into_iter().collect()
}

fn derive_candidate_indices(
    total_cards: usize,
    term_groups: &[Vec<String>],
    term_index: &HashMap<String, RoaringBitmap>,
) -> Vec<usize> {
    let mut candidates: Option<RoaringBitmap> = None;

    for group in term_groups {
        let indexable_tokens = group
            .iter()
            .map(|token| token.trim())
            .filter(|token| is_indexable_term(token))
            .collect::<Vec<_>>();

        if indexable_tokens.is_empty() {
            continue;
        }

        let mut group_union = RoaringBitmap::new();
        for token in indexable_tokens {
            for (key, postings) in term_index.iter() {
                if key.contains(token) {
                    group_union |= postings;
                }
            }
        }

        if group_union.is_empty() {
            return Vec::new();
        }

        candidates = Some(match candidates {
            Some(previous) => previous & group_union,
            None => group_union,
        });

        if let Some(current) = &candidates {
            if current.is_empty() {
                return Vec::new();
            }
        }
    }

    match candidates {
        Some(bitmap) => bitmap.iter().map(|value| value as usize).collect(),
        None => (0..total_cards).collect(),
    }
}

fn prune_query_state_cache(cache: &mut HashMap<String, QueryStateEntry>) {
    while cache.len() > MAX_QUERY_STATE_ENTRIES_PER_SHARD {
        if let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_used_ms)
            .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest_key);
        } else {
            break;
        }
    }
}

fn prune_query_result_cache(cache: &mut HashMap<String, QueryResultCacheEntry>, now_ms: u128) {
    cache.retain(|_, entry| entry.expires_at_ms > now_ms);

    while cache.len() > MAX_QUERY_RESULT_ENTRIES_PER_SHARD {
        if let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_used_ms)
            .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest_key);
        } else {
            break;
        }
    }
}

fn shard_index_for_key(key: &str, shard_count: usize) -> usize {
    if shard_count <= 1 {
        return 0;
    }

    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    (hasher.finish() as usize) % shard_count
}

fn empty_query_state_shards() -> Vec<Mutex<HashMap<String, QueryStateEntry>>> {
    (0..QUERY_STATE_SHARDS)
        .map(|_| Mutex::new(HashMap::new()))
        .collect()
}

fn empty_query_result_shards() -> Vec<Mutex<HashMap<String, QueryResultCacheEntry>>> {
    (0..QUERY_RESULT_SHARDS)
        .map(|_| Mutex::new(HashMap::new()))
        .collect()
}

fn normalize_search_signature(value: &str) -> String {
    value
        .split_whitespace()
        .map(|part| part.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_csv_signature(value: &str) -> String {
    let mut values = shared_core::split_csv(value);
    values.sort();
    values.dedup();
    values.join(",")
}

fn normalized_query_result_key(
    params: &QueryParams,
    sort_by: &str,
    cite_match: &str,
    match_mode: &str,
    limit: usize,
    cursor: usize,
) -> String {
    [
        normalize_search_signature(&params.search),
        params.start_date.trim().to_string(),
        params.end_date.trim().to_string(),
        normalize_csv_signature(&params.exclude_sides),
        normalize_csv_signature(&params.exclude_division),
        normalize_csv_signature(&params.exclude_schools),
        normalize_csv_signature(&params.exclude_years),
        sort_by.to_string(),
        cite_match.to_string(),
        match_mode.to_string(),
        limit.to_string(),
        cursor.to_string(),
    ]
    .join("\u{241f}")
}

async fn get_cached_query_response(
    state: &AppState,
    key: &str,
    signature: &IndexSignature,
) -> Option<Value> {
    let shard_idx = shard_index_for_key(key, state.query_result_shards.len());
    let now_ms = now_millis();
    let mut shard = state.query_result_shards[shard_idx].lock().await;
    prune_query_result_cache(&mut shard, now_ms);

    let entry = shard.get_mut(key)?;
    if &entry.signature != signature {
        return None;
    }

    entry.last_used_ms = now_ms;
    Some(entry.payload.clone())
}

async fn store_cached_query_response(state: &AppState, key: &str, signature: IndexSignature, payload: Value) {
    let shard_idx = shard_index_for_key(key, state.query_result_shards.len());
    let now_ms = now_millis();
    let mut shard = state.query_result_shards[shard_idx].lock().await;
    shard.insert(
        key.to_string(),
        QueryResultCacheEntry {
            signature,
            payload,
            expires_at_ms: now_ms + QUERY_RESULT_CACHE_TTL_MS,
            last_used_ms: now_ms,
        },
    );
    prune_query_result_cache(&mut shard, now_ms);
}

async fn warm_index_caches_on_startup(state: &AppState) {
    let _ = get_cards_cache_snapshot(state).await;
}

fn append_parser_event(state: &AppState, level: &str, message: &str, payload: Option<Value>) {
    let mut event = json!({
        "id": format!("{}-{}", now_millis(), std::process::id()),
        "at": now_millis(),
        "level": level,
        "message": message,
    });

    if let Some(Value::Object(extra)) = payload {
        if let Value::Object(map) = &mut event {
            for (key, value) in extra {
                map.insert(key, value);
            }
        }
    }

    ensure_parent_dir(&state.parser_events_path);

    let line = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
    let existing = fs::read_to_string(&state.parser_events_path).unwrap_or_default();
    let next = if existing.is_empty() {
        format!("{}\n", line)
    } else {
        format!("{}{}\n", existing, line)
    };
    let _ = fs::write(&state.parser_events_path, next);
}

fn find_uploaded_docx_file(local_docs_folder: &Path, filename: &str) -> Option<UploadedDoc> {
    let normalized = filename.trim().to_ascii_lowercase();
    list_uploaded_docx(local_docs_folder)
        .into_iter()
        .find(|item| item.filename.trim().to_ascii_lowercase() == normalized)
}

fn dedupe_target_path(dir: &Path, filename: &str) -> PathBuf {
    let base = Path::new(filename)
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "upload".to_string());
    let extension = Path::new(filename)
        .extension()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "docx".to_string());

    let mut candidate = dir.join(filename);
    let mut suffix = 1_u32;
    while candidate.exists() {
        candidate = dir.join(format!("{}-{}.{}", base, suffix, extension));
        suffix += 1;
    }

    candidate
}

fn sanitize_filename(filename: &str) -> String {
    let mut out = String::with_capacity(filename.len());
    for ch in filename.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }

    let trimmed = out.trim_matches('.').trim().to_string();
    if trimmed.is_empty() {
        "upload.docx".to_string()
    } else {
        trimmed
    }
}

fn parse_bool_str(value: &str, fallback: bool) -> bool {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => fallback,
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn load_cards(index_path: &Path) -> Vec<Card> {
    let content = match fs::read_to_string(index_path) {
        Ok(value) => value,
        Err(error) if error.kind() == ErrorKind::NotFound => return Vec::new(),
        Err(_) => return Vec::new(),
    };

    if content.trim().is_empty() {
        return Vec::new();
    }

    let stripped = content.trim_start();
    if stripped.starts_with('[') {
        if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&content) {
            return items
                .into_iter()
                .filter_map(|item| match item {
                    Value::Object(map) => Some(map),
                    _ => None,
                })
                .collect();
        }
        return Vec::new();
    }

    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            serde_json::from_str::<Value>(trimmed).ok().and_then(|value| match value {
                Value::Object(map) => Some(map),
                _ => None,
            })
        })
        .collect()
}

fn write_cards_jsonl(index_path: &Path, cards: &[Card]) -> std::io::Result<()> {
    ensure_parent_dir(index_path);

    let mut lines = String::new();
    for card in cards {
        lines.push_str(&serde_json::to_string(card).unwrap_or_else(|_| "{}".to_string()));
        lines.push('\n');
    }

    fs::write(index_path, lines)
}

fn string_field(card: &Card, key: &str) -> String {
    card.get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}


static SYNONYM_LOOKUP: Lazy<HashMap<String, Vec<String>>> = Lazy::new(load_synonym_lookup);

fn load_synonym_lookup() -> HashMap<String, Vec<String>> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(path) = env::var("SYNONYMS_PATH") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }

    candidates.push(PathBuf::from("./synonyms.txt"));
    candidates.push(PathBuf::from("../synonyms.txt"));
    candidates.push(PathBuf::from("../../synonyms.txt"));
    candidates.push(PathBuf::from("../rust-backend/synonyms.txt"));
    candidates.push(PathBuf::from("../../rust-backend/synonyms.txt"));
    candidates.push(PathBuf::from("../verbatim-parser /synonyms.txt"));
    candidates.push(PathBuf::from("../verbatim-parser/synonyms.txt"));

    let mut content = String::new();
    for path in candidates {
        if let Ok(text) = fs::read_to_string(&path) {
            content = text;
            break;
        }
    }

    if content.trim().is_empty() {
        content = include_str!("../synonyms.txt").to_string();
    }

    if content.trim().is_empty() {
        return HashMap::new();
    }

    let mut lookup: HashMap<String, Vec<String>> = HashMap::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut aliases: Vec<String> = Vec::new();
        for value in line.split(',') {
            let normalized = value.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                continue;
            }
            if !aliases.contains(&normalized) {
                aliases.push(normalized);
            }
        }

        if aliases.len() < 2 {
            continue;
        }

        for alias in &aliases {
            lookup.insert(alias.clone(), aliases.clone());
        }
    }

    lookup
}

fn synonym_group(token: &str) -> Vec<String> {
    if token.is_empty() {
        return Vec::new();
    }

    let lowered = token.trim().to_ascii_lowercase();
    if lowered.is_empty() {
        return Vec::new();
    }

    if let Some(group) = SYNONYM_LOOKUP.get(&lowered) {
        return group.clone();
    }

    vec![lowered]
}

fn expand_query_with_synonyms(terms: &[String], phrases: &[String]) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let expanded_terms = terms.iter().map(|term| synonym_group(term)).collect();
    let expanded_phrases = phrases.iter().map(|phrase| synonym_group(phrase)).collect();
    (expanded_terms, expanded_phrases)
}

fn normalize_query_text(value: &str) -> String {
    let mut normalized = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch.is_whitespace() {
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(' ');
        }
    }

    normalized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn tag_priority_match(card: &Card, term_groups: &[Vec<String>], phrase_groups: &[Vec<String>]) -> bool {
    if term_groups.is_empty() && phrase_groups.is_empty() {
        return false;
    }

    let tag = string_field(card, "tag");
    let tag_base = string_field(card, "tag_base");
    let candidate = format!(
        "{} {} {} {}",
        if tag.is_empty() { tag_base } else { tag },
        string_field(card, "card_identifier"),
        string_field(card, "card_identifier_token"),
        string_field(card, "card_number")
    );

    let normalized = normalize_query_text(&candidate);
    phrase_groups.iter().all(|group| {
        group
            .iter()
            .any(|phrase| normalized.contains(&normalize_query_text(phrase)))
    }) && term_groups.iter().all(|group| {
        group
            .iter()
            .any(|term| normalized.contains(&normalize_query_text(term)))
    })
}

fn paragraph_match(card: &Card, term_groups: &[Vec<String>], phrase_groups: &[Vec<String>]) -> bool {
    if term_groups.is_empty() && phrase_groups.is_empty() {
        return true;
    }

    let body_text = card
        .get("body")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| {
            card.get("body")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string()
        });

    let paragraph = format!("{} {}", string_field(card, "highlighted_text"), body_text).to_ascii_lowercase();

    phrase_groups
        .iter()
        .all(|group| group.iter().any(|phrase| paragraph.contains(phrase)))
        && term_groups
            .iter()
            .all(|group| group.iter().any(|term| paragraph.contains(term)))
}

fn card_search_blob(card: &Card) -> String {
    let body_text = card
        .get("body")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| {
            card.get("body")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string()
        });

    format!(
        "{} {} {} {} {} {} {}",
        string_field(card, "tag"),
        string_field(card, "card_identifier"),
        string_field(card, "card_identifier_token"),
        string_field(card, "card_number"),
        string_field(card, "highlighted_text"),
        string_field(card, "cite"),
        body_text,
    )
    .to_ascii_lowercase()
}

#[allow(clippy::too_many_arguments)]
fn card_matches(
    card: &Card,
    query_card: &QueryCardCache,
    term_groups: &[Vec<String>],
    phrase_groups: &[Vec<String>],
    excluded_sides: &HashSet<String>,
    excluded_divisions: &HashSet<String>,
    excluded_years: &HashSet<String>,
    excluded_schools: &HashSet<String>,
    start_ts: Option<i64>,
    end_ts: Option<i64>,
    cite_match: &str,
    match_mode: &str,
) -> bool {
    if !excluded_sides.is_empty()
        && excluded_sides
            .iter()
            .any(|side| query_card.filename_lc.contains(side))
    {
        return false;
    }

    if !excluded_divisions.is_empty() && excluded_divisions.contains(query_card.division_lc.trim()) {
        return false;
    }

    if !excluded_years.is_empty() && excluded_years.contains(query_card.year_lc.trim()) {
        return false;
    }

    if !excluded_schools.is_empty() && excluded_schools.contains(query_card.school_lc.trim()) {
        return false;
    }

    if let (Some(start), Some(end)) = (start_ts, end_ts) {
        match query_card.cite_ts {
            Some(card_ts) if card_ts >= start && card_ts <= end => {}
            _ => return false,
        }
    }

    if !cite_match.is_empty() && !query_card.cite_lc.contains(cite_match) {
        return false;
    }

    if match_mode == "tag" {
        return tag_priority_match(card, term_groups, phrase_groups);
    }

    if match_mode == "paragraph" {
        return paragraph_match(card, term_groups, phrase_groups);
    }

    phrase_groups
        .iter()
        .all(|group| group.iter().any(|phrase| query_card.search_blob_lc.contains(phrase)))
        && term_groups
            .iter()
            .all(|group| group.iter().any(|term| query_card.search_blob_lc.contains(term)))
}

fn card_timestamp(card: &Card) -> Option<i64> {
    let value = card.get("cite_date")?;
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => shared_core::to_unix_timestamp(text),
        _ => None,
    }
}

fn ensure_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
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

#[derive(Debug)]
struct UploadedDoc {
    filename: String,
    relative_path: String,
    absolute_path: PathBuf,
}

fn list_uploaded_docx(local_docs_folder: &Path) -> Vec<UploadedDoc> {
    let upload_root = local_docs_folder.join("uploaded_docs");
    if !upload_root.is_dir() {
        return Vec::new();
    }

    WalkDir::new(&upload_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let absolute_path = entry.path().to_path_buf();
            let filename = absolute_path.file_name()?.to_string_lossy().to_string();
            if filename.starts_with("~$") || !filename.to_ascii_lowercase().ends_with(".docx") {
                return None;
            }

            let relative_path = absolute_path
                .strip_prefix(&upload_root)
                .ok()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| filename.clone());

            Some(UploadedDoc {
                filename,
                relative_path,
                absolute_path,
            })
        })
        .collect()
}

fn discover_docx_files(local_docs_folder: &Path) -> Vec<PathBuf> {
    if !local_docs_folder.is_dir() {
        return Vec::new();
    }

    let mut files = WalkDir::new(local_docs_folder)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| {
            let name = path
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_default();
            !name.starts_with("~$") && name.to_ascii_lowercase().ends_with(".docx")
        })
        .collect::<Vec<_>>();

    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn unique_test_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "logos-rust-tests-{}-{}-{}",
            label,
            std::process::id(),
            now_millis()
        ));
        fs::create_dir_all(&root).expect("create test root");
        root
    }

    fn make_test_state(root: &Path) -> Arc<AppState> {
        let local_docs_folder = root.join("local_docs");
        fs::create_dir_all(local_docs_folder.join("uploaded_docs")).expect("create uploaded_docs");

        let local_index_path = local_docs_folder.join("cards_index.json");
        let index_cache_path = local_docs_folder.join("cards_cache.bin");
        let parser_settings_path = local_docs_folder.join("parser_settings.json");
        let parser_events_path = local_docs_folder.join("parser_events.jsonl");

        fs::write(&local_index_path, "").expect("init empty index");
        fs::write(&index_cache_path, "").expect("init empty cache");
        fs::write(&parser_settings_path, "{}").expect("init parser settings");
        fs::write(&parser_events_path, "").expect("init parser events");

        let (parse_job_sender, _parse_job_receiver) = mpsc::channel::<ParseJob>(8);

        Arc::new(AppState {
            local_docs_folder,
            local_index_path,
            index_cache_path,
            parser_settings_path,
            parser_events_path,
            index_lock: Arc::new(Mutex::new(())),
            active_upload_parses: Arc::new(Mutex::new(HashSet::new())),
            parse_job_sender,
            cards_cache: Arc::new(Mutex::new(CardsCache {
                signature: IndexSignature::empty(),
                cards: Arc::new(Vec::new()),
                query_cards: Arc::new(Vec::new()),
                term_index: Arc::new(HashMap::new()),
                id_lookup: Arc::new(HashMap::new()),
                filename_counts: Arc::new(HashMap::new()),
                schools: Arc::new(BTreeSet::new()),
                indexed_filenames: Arc::new(HashSet::new()),
            })),
            query_state_shards: Arc::new(empty_query_state_shards()),
            query_result_shards: Arc::new(empty_query_result_shards()),
        })
    }

    fn make_card(id: &str, filename: &str, school: &str, body: &str) -> Card {
        let mut card = Card::new();
        card.insert("id".to_string(), Value::String(id.to_string()));
        card.insert("filename".to_string(), Value::String(filename.to_string()));
        card.insert("school".to_string(), Value::String(school.to_string()));
        card.insert("body".to_string(), Value::String(body.to_string()));
        card.insert("tag".to_string(), Value::String(body.to_string()));
        card.insert("cite".to_string(), Value::String("cite".to_string()));
        card.insert("division".to_string(), Value::String("open".to_string()));
        card.insert("year".to_string(), Value::String("24-25".to_string()));
        card
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response bytes");
        serde_json::from_slice(&bytes).expect("json body")
    }

    #[tokio::test]
    async fn parity_query_is_incremental_paginated() {
        let root = unique_test_root("query");
        let state = make_test_state(&root);

        let cards = (0..100)
            .map(|idx| make_card(&format!("id-{idx}"), "alpha.docx", "SchoolA", "alpha evidence"))
            .collect::<Vec<_>>();
        write_cards_jsonl(&state.local_index_path, &cards).expect("write cards");
        refresh_index_caches(&state).await;

        let response_page1 = query_cards(
            State(state.clone()),
            Query(QueryParams {
                search: "alpha".to_string(),
                cursor: 0,
                start_date: String::new(),
                end_date: String::new(),
                exclude_sides: String::new(),
                exclude_division: String::new(),
                exclude_schools: String::new(),
                exclude_years: String::new(),
                sort_by: String::new(),
                cite_match: String::new(),
                limit: 30,
                match_mode: String::new(),
            }),
        )
        .await
        .into_response();

        let page1 = response_json(response_page1).await;
        assert_eq!(page1["count"], 30);
        assert_eq!(page1["cursor"], 30);
        assert_eq!(page1["has_more"], true);
        assert_eq!(page1["count_is_partial"], true);
        assert_eq!(page1["total_count"], 30);

        let response_page4 = query_cards(
            State(state.clone()),
            Query(QueryParams {
                search: "alpha".to_string(),
                cursor: 90,
                start_date: String::new(),
                end_date: String::new(),
                exclude_sides: String::new(),
                exclude_division: String::new(),
                exclude_schools: String::new(),
                exclude_years: String::new(),
                sort_by: String::new(),
                cite_match: String::new(),
                limit: 30,
                match_mode: String::new(),
            }),
        )
        .await
        .into_response();

        let page4 = response_json(response_page4).await;
        assert_eq!(page4["count"], 10);
        assert_eq!(page4["cursor"], 100);
        assert_eq!(page4["has_more"], false);
        assert_eq!(page4["count_is_partial"], false);
        assert_eq!(page4["total_count"], 100);
    }

    #[tokio::test]
    async fn parity_card_returns_object_or_null() {
        let root = unique_test_root("card");
        let state = make_test_state(&root);

        write_cards_jsonl(
            &state.local_index_path,
            &[make_card("id-1", "alpha.docx", "SchoolA", "alpha")],
        )
        .expect("write cards");
        refresh_index_caches(&state).await;

        let found = get_card(
            State(state.clone()),
            Query(CardParams {
                id: "id-1".to_string(),
            }),
        )
        .await
        .into_response();
        let found_json = response_json(found).await;
        assert_eq!(found_json["id"], "id-1");

        let missing = get_card(
            State(state.clone()),
            Query(CardParams {
                id: "missing".to_string(),
            }),
        )
        .await
        .into_response();
        let missing_json = response_json(missing).await;
        assert_eq!(missing_json, Value::Null);
    }

    #[tokio::test]
    async fn parity_documents_merge_index_and_folder() {
        let root = unique_test_root("documents");
        let state = make_test_state(&root);

        let cards = vec![
            make_card("id-1", "alpha.docx", "SchoolA", "alpha"),
            make_card("id-2", "alpha.docx", "SchoolA", "alpha2"),
            make_card("id-3", "beta.docx", "SchoolB", "beta"),
        ];
        write_cards_jsonl(&state.local_index_path, &cards).expect("write cards");
        fs::write(
            state.local_docs_folder.join("uploaded_docs").join("beta.docx"),
            b"test",
        )
        .expect("write beta upload");
        fs::write(
            state.local_docs_folder.join("uploaded_docs").join("gamma.docx"),
            b"test",
        )
        .expect("write gamma upload");
        refresh_index_caches(&state).await;

        let response = get_documents(State(state.clone())).await.into_response();
        let payload = response_json(response).await;
        let docs = payload["documents"].as_array().expect("documents array");

        let mut by_name: HashMap<String, Value> = HashMap::new();
        for doc in docs {
            let name = doc["filename"].as_str().unwrap_or_default().to_string();
            by_name.insert(name, doc.clone());
        }

        assert_eq!(by_name["alpha.docx"]["cards_indexed"], 2);
        assert_eq!(by_name["alpha.docx"]["in_index"], true);
        assert_eq!(by_name["alpha.docx"]["in_folder"], false);

        assert_eq!(by_name["beta.docx"]["cards_indexed"], 1);
        assert_eq!(by_name["beta.docx"]["in_index"], true);
        assert_eq!(by_name["beta.docx"]["in_folder"], true);

        assert_eq!(by_name["gamma.docx"]["cards_indexed"], 0);
        assert_eq!(by_name["gamma.docx"]["in_index"], false);
        assert_eq!(by_name["gamma.docx"]["in_folder"], true);
    }

    #[tokio::test]
    async fn parity_delete_document_index_and_folder_paths() {
        let root = unique_test_root("delete");
        let state = make_test_state(&root);

        write_cards_jsonl(
            &state.local_index_path,
            &[make_card("id-del", "remove-me.docx", "SchoolA", "alpha")],
        )
        .expect("write cards");
        let folder_file = state.local_docs_folder.join("uploaded_docs").join("remove-file.docx");
        fs::write(&folder_file, b"test").expect("write upload file");
        refresh_index_caches(&state).await;

        let delete_index = delete_document(
            State(state.clone()),
            Json(DeleteDocumentRequest {
                filename: "remove-me.docx".to_string(),
                target: "index".to_string(),
            }),
        )
        .await
        .into_response();
        assert_eq!(delete_index.status(), StatusCode::OK);
        let delete_index_json = response_json(delete_index).await;
        assert_eq!(delete_index_json["removed_cards"], 1);

        let delete_folder = delete_document(
            State(state.clone()),
            Json(DeleteDocumentRequest {
                filename: "remove-file.docx".to_string(),
                target: "folder".to_string(),
            }),
        )
        .await
        .into_response();
        assert_eq!(delete_folder.status(), StatusCode::OK);
        let delete_folder_json = response_json(delete_folder).await;
        assert_eq!(delete_folder_json["removed_from_folder"], true);
        assert!(!folder_file.exists());
    }

    #[tokio::test]
    async fn parity_index_document_edge_errors() {
        let root = unique_test_root("index-document");
        let state = make_test_state(&root);

        let missing_filename = index_document(
            State(state.clone()),
            Json(IndexDocumentRequest {
                filename: "".to_string(),
            }),
        )
        .await
        .into_response();
        assert_eq!(missing_filename.status(), StatusCode::BAD_REQUEST);

        let not_found = index_document(
            State(state.clone()),
            Json(IndexDocumentRequest {
                filename: "not-there.docx".to_string(),
            }),
        )
        .await
        .into_response();
        assert_eq!(not_found.status(), StatusCode::NOT_FOUND);
    }
}
