use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub type Card = Map<String, Value>;

#[derive(Debug, Clone, Deserialize)]
pub struct QueryParams {
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub cursor: usize,
    #[serde(default)]
    pub start_date: String,
    #[serde(default)]
    pub end_date: String,
    #[serde(default)]
    pub exclude_sides: String,
    #[serde(default)]
    pub exclude_division: String,
    #[serde(default)]
    pub exclude_schools: String,
    #[serde(default)]
    pub exclude_years: String,
    #[serde(default)]
    pub sort_by: String,
    #[serde(default)]
    pub cite_match: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub match_mode: String,
}

fn default_limit() -> usize {
    30
}

pub fn load_cards(index_path: &Path) -> Vec<Card> {
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
            serde_json::from_str::<Value>(trimmed)
                .ok()
                .and_then(|value| match value {
                    Value::Object(map) => Some(map),
                    _ => None,
                })
        })
        .collect()
}

pub fn get_card(cards: &[Card], id: &str) -> Value {
    cards
        .iter()
        .find(|card| string_field(card, "id") == id)
        .cloned()
        .map(Value::Object)
        .unwrap_or(Value::Null)
}

pub fn get_schools(cards: &[Card]) -> Value {
    let mut schools = BTreeSet::new();
    for card in cards {
        let school = string_field(card, "school").trim().to_string();
        if !school.is_empty() {
            schools.insert(school);
        }
    }

    serde_json::json!({
        "colleges": schools.into_iter().collect::<Vec<_>>()
    })
}

pub fn query_cards(cards: &[Card], params: &QueryParams) -> Value {
    let safe_limit = params.limit.clamp(1, 30);
    let offset = params.cursor;
    let (terms, phrases) = split_query_terms(&params.search);
    let (term_groups, phrase_groups) = expand_query_with_synonyms(&terms, &phrases);

    let excluded_sides: HashSet<String> = split_csv(&params.exclude_sides).into_iter().collect();
    let excluded_divisions: HashSet<String> = split_csv(&params.exclude_division)
        .into_iter()
        .map(|value| value.split('-').next().unwrap_or("").trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    let excluded_years: HashSet<String> = split_csv(&params.exclude_years).into_iter().collect();
    let excluded_schools: HashSet<String> = split_csv(&params.exclude_schools).into_iter().collect();

    let normalized_sort_by = params.sort_by.trim().to_ascii_lowercase();
    let normalized_match_mode = params.match_mode.trim().to_ascii_lowercase();
    let cite_match = params.cite_match.trim().to_ascii_lowercase();
    let start_ts = to_unix_timestamp(&params.start_date);
    let end_ts = to_unix_timestamp(&params.end_date);

    // Early termination: for non-date-sorted queries, stop once we have enough matches
    let needs_full_scan = normalized_sort_by == "date";
    let early_stop_count = offset + safe_limit + 1; // +1 to know if has_more

    let mut matched_indices = Vec::new();
    for (index, card) in cards.iter().enumerate() {
        if card_matches(
            card,
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
            matched_indices.push(index);
            
            // Early termination for non-sorted queries
            if !needs_full_scan && matched_indices.len() >= early_stop_count {
                break;
            }
        }
    }

    if normalized_sort_by == "date" {
        matched_indices.sort_by(|a, b| {
            let a_ts = card_timestamp(&cards[*a]).unwrap_or(0);
            let b_ts = card_timestamp(&cards[*b]).unwrap_or(0);
            b_ts.cmp(&a_ts)
        });
    }

    let total_count = matched_indices.len();
    let count_is_partial = !needs_full_scan && total_count >= early_stop_count;
    
    let page_results: Vec<Value> = matched_indices
        .iter()
        .copied()
        .skip(offset)
        .take(safe_limit)
        .filter_map(|index| cards.get(index).map(card_to_search_result))
        .collect();

    let cursor = offset + page_results.len();
    // has_more is true if there are more matches beyond the current page
    let has_more = matched_indices.len() > offset + page_results.len();

    serde_json::json!({
        "count": page_results.len(),
        "results": page_results,
        "cursor": cursor,
        "total_count": total_count,
        "has_more": has_more,
        "count_is_partial": count_is_partial
    })
}

pub fn clear_index_file(index_path: &Path) -> Result<(), String> {
    if let Some(parent) = index_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    fs::write(index_path, "").map_err(|err| err.to_string())
}

pub fn delete_document(index_path: &Path, local_docs_folder: &Path, filename: &str, target: &str) -> Result<Value, String> {
    let normalized_filename = filename.trim();
    if normalized_filename.is_empty() {
        return Err("filename is required".to_string());
    }

    let normalized_target = target.trim().to_ascii_lowercase();
    if normalized_target != "index" && normalized_target != "folder" {
        return Err("target must be either 'index' or 'folder'".to_string());
    }

    let mut removed_cards = 0_u64;
    let mut removed_from_folder = false;
    let mut deleted_path: Option<String> = None;

    if normalized_target == "index" {
        let cards = load_cards(index_path);
        let needle = normalized_filename.to_ascii_lowercase();

        let mut kept = Vec::new();
        for card in cards {
            let card_filename = string_field(&card, "filename").trim().to_ascii_lowercase();
            if card_filename == needle {
                removed_cards += 1;
            } else {
                kept.push(card);
            }
        }

        write_cards_jsonl(index_path, &kept).map_err(|err| err.to_string())?;
    } else {
        let needle = normalized_filename.to_ascii_lowercase();
        let uploaded = list_uploaded_docx(local_docs_folder);

        if let Some(item) = uploaded
            .into_iter()
            .find(|candidate| candidate.filename.to_ascii_lowercase() == needle)
        {
            fs::remove_file(&item.absolute_path).map_err(|err| err.to_string())?;
            removed_from_folder = true;
            deleted_path = Some(item.relative_path);
        }
    }

    if removed_cards == 0 && !removed_from_folder {
        return Err("Document not found for selected target".to_string());
    }

    Ok(serde_json::json!({
        "ok": true,
        "removed_cards": removed_cards,
        "removed_from_folder": removed_from_folder,
        "deleted_path": deleted_path
    }))
}

pub fn get_documents(cards: &[Card], local_docs_folder: &Path) -> Value {
    let mut filename_counts: HashMap<String, (String, u64)> = HashMap::new();
    for card in cards {
        let filename = string_field(card, "filename").trim().to_string();
        if filename.is_empty() {
            continue;
        }

        let key = filename.to_ascii_lowercase();
        let entry = filename_counts.entry(key).or_insert((filename, 0));
        entry.1 += 1;
    }

    let uploaded = list_uploaded_docx(local_docs_folder);
    let mut uploaded_by_name: HashMap<String, (String, String)> = HashMap::new();
    for item in uploaded {
        uploaded_by_name.insert(
            item.filename.to_ascii_lowercase(),
            (item.filename, item.relative_path),
        );
    }

    let mut keys: Vec<String> = filename_counts
        .keys()
        .chain(uploaded_by_name.keys())
        .cloned()
        .collect();
    keys.sort();
    keys.dedup();

    let mut documents = Vec::with_capacity(keys.len());
    for key in keys {
        let indexed = filename_counts.get(&key);
        let uploaded = uploaded_by_name.get(&key);

        let filename = indexed
            .map(|value| value.0.clone())
            .or_else(|| uploaded.map(|value| value.0.clone()))
            .unwrap_or_default();

        documents.push(serde_json::json!({
            "filename": filename,
            "cards_indexed": indexed.map(|value| value.1).unwrap_or(0),
            "in_index": indexed.is_some(),
            "in_folder": uploaded.is_some(),
            "folder_path": uploaded.map(|value| value.1.clone())
        }));
    }

    serde_json::json!({ "documents": documents })
}

pub fn card_to_search_result(card: &Card) -> Value {
    let mut out = Map::new();

    let copy_keys = [
        "id",
        "tag",
        "tag_sub",
        "tag_base",
        "card_number",
        "card_identifier",
        "card_identifier_token",
        "cite",
        "division",
        "s3_url",
        "year",
        "download_url",
        "cite_emphasis",
    ];

    for key in copy_keys {
        if let Some(value) = card.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }

    Value::Object(out)
}

#[derive(Debug)]
struct UploadedDoc {
    filename: String,
    relative_path: String,
    absolute_path: std::path::PathBuf,
}

fn list_uploaded_docx(local_docs_folder: &Path) -> Vec<UploadedDoc> {
    let upload_root = local_docs_folder.join("uploaded_docs");
    if !upload_root.is_dir() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut stack = vec![upload_root.clone()];

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            if !path.is_file() {
                continue;
            }

            let filename = match path.file_name() {
                Some(value) => value.to_string_lossy().to_string(),
                None => continue,
            };

            if filename.starts_with("~$") || !filename.to_ascii_lowercase().ends_with(".docx") {
                continue;
            }

            let relative_path = path
                .strip_prefix(&upload_root)
                .ok()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| filename.clone());

            out.push(UploadedDoc {
                filename,
                relative_path,
                absolute_path: path,
            });
        }
    }

    out
}

fn card_matches(
    card: &Card,
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
    let filename_lc = string_field(card, "filename").to_ascii_lowercase();
    let division_lc = string_field(card, "division")
        .split('-')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let year_lc = string_field(card, "year").trim().to_ascii_lowercase();
    let school_lc = string_field(card, "school").trim().to_ascii_lowercase();
    let cite_lc = string_field(card, "cite").to_ascii_lowercase();

    if !excluded_sides.is_empty() && excluded_sides.iter().any(|side| filename_lc.contains(side)) {
        return false;
    }

    if !excluded_divisions.is_empty() && excluded_divisions.contains(division_lc.trim()) {
        return false;
    }

    if !excluded_years.is_empty() && excluded_years.contains(year_lc.trim()) {
        return false;
    }

    if !excluded_schools.is_empty() && excluded_schools.contains(school_lc.trim()) {
        return false;
    }

    if let (Some(start), Some(end)) = (start_ts, end_ts) {
        match card_timestamp(card) {
            Some(card_ts) if card_ts >= start && card_ts <= end => {}
            _ => return false,
        }
    }

    if !cite_match.is_empty() && !cite_lc.contains(cite_match) {
        return false;
    }

    if match_mode == "tag" {
        let normalized = normalize_query_text(&format!(
            "{} {} {} {}",
            {
                let tag = string_field(card, "tag");
                if tag.is_empty() {
                    string_field(card, "tag_base")
                } else {
                    tag
                }
            },
            string_field(card, "card_identifier"),
            string_field(card, "card_identifier_token"),
            string_field(card, "card_number")
        ));

        return phrase_groups.iter().all(|group| {
            group
                .iter()
                .any(|phrase| normalized.contains(&normalize_query_text(phrase)))
        }) && term_groups.iter().all(|group| {
            group
                .iter()
                .any(|term| normalized.contains(&normalize_query_text(term)))
        });
    }

    if match_mode == "paragraph" {
        let body_text = body_text(card);
        let paragraph = format!("{} {}", string_field(card, "highlighted_text"), body_text)
            .to_ascii_lowercase();

        return phrase_groups
            .iter()
            .all(|group| group.iter().any(|phrase| paragraph.contains(phrase)))
            && term_groups
                .iter()
                .all(|group| group.iter().any(|term| paragraph.contains(term)));
    }

    let search_blob_lc = card_search_blob(card);
    phrase_groups
        .iter()
        .all(|group| group.iter().any(|phrase| search_blob_lc.contains(phrase)))
        && term_groups
            .iter()
            .all(|group| group.iter().any(|term| search_blob_lc.contains(term)))
}

static SYNONYM_LOOKUP: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();

fn synonym_lookup() -> &'static HashMap<String, Vec<String>> {
    SYNONYM_LOOKUP.get_or_init(load_synonym_lookup)
}

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
        content = include_str!("../../../synonyms.txt").to_string();
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

    if let Some(group) = synonym_lookup().get(&lowered) {
        return group.clone();
    }

    vec![lowered]
}

fn expand_query_with_synonyms(terms: &[String], phrases: &[String]) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let expanded_terms = terms.iter().map(|term| synonym_group(term)).collect();
    let expanded_phrases = phrases.iter().map(|phrase| synonym_group(phrase)).collect();
    (expanded_terms, expanded_phrases)
}

fn body_text(card: &Card) -> String {
    card.get("body")
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
        })
}

fn card_search_blob(card: &Card) -> String {
    format!(
        "{} {} {} {} {} {} {}",
        string_field(card, "tag"),
        string_field(card, "card_identifier"),
        string_field(card, "card_identifier_token"),
        string_field(card, "card_number"),
        string_field(card, "highlighted_text"),
        string_field(card, "cite"),
        body_text(card),
    )
    .to_ascii_lowercase()
}

fn string_field(card: &Card, key: &str) -> String {
    card.get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

pub fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|item| item.trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .collect()
}

pub fn split_query_terms(query: &str) -> (Vec<String>, Vec<String>) {
    let mut phrases = Vec::new();
    let mut remainder = String::new();
    let chars = query.chars();
    let mut in_quotes = false;
    let mut current_phrase = String::new();

    for ch in chars {
        if ch == '"' {
            if in_quotes {
                let phrase = current_phrase.trim().to_ascii_lowercase();
                if !phrase.is_empty() {
                    phrases.push(phrase);
                }
                current_phrase.clear();
            }
            in_quotes = !in_quotes;
            continue;
        }

        if in_quotes {
            current_phrase.push(ch);
        } else {
            remainder.push(ch);
        }
    }

    let terms = remainder
        .split_whitespace()
        .map(|item| item.trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .collect();

    (terms, phrases)
}

fn write_cards_jsonl(index_path: &Path, cards: &[Card]) -> std::io::Result<()> {
    if let Some(parent) = index_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut lines = String::new();
    for card in cards {
        lines.push_str(&serde_json::to_string(card).unwrap_or_else(|_| "{}".to_string()));
        lines.push('\n');
    }

    fs::write(index_path, lines)
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

pub fn to_unix_timestamp(raw: &str) -> Option<i64> {
    if raw.trim().is_empty() {
        return None;
    }

    if let Ok(value) = raw.trim().parse::<f64>() {
        return Some(value as i64);
    }

    for format in ["%Y-%m-%d", "%Y/%m/%d"] {
        if let Ok(date) = NaiveDate::parse_from_str(raw.trim(), format) {
            return date.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc().timestamp());
        }
    }

    None
}

fn card_timestamp(card: &Card) -> Option<i64> {
    let value = card.get("cite_date")?;
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => to_unix_timestamp(text),
        _ => None,
    }
}
