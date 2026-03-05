use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use logos_backend::app_config::BackendPaths;
use logos_backend::{
    backend_executable_from_env, ensure_uploaded_copy, index_uploaded_filename_via_backend, BridgeMode,
};
use logos_core::load_cards;
use reqwest::multipart::{Form, Part};
use scraper::{Html, Selector};
use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub struct ScraperConfig {
    pub wiki_url: String,
    pub division: String,
    pub year: String,
    pub tmp_folder: PathBuf,
    pub backend_api_url: String,
    pub bridge_mode: BridgeMode,
    pub backend_executable: PathBuf,
    pub backend_paths: BackendPaths,
    pub check_existing: bool,
    pub skip_scrape_when_urls_exist: bool,
}

fn absolute_url(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else {
        format!("{}{}", base.trim_end_matches('/'), href)
    }
}

fn extract_docx_links(html: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let document = Html::parse_document(html);
    let selector = match Selector::parse("span.wikiexternallink a[href]") {
        Ok(v) => v,
        Err(_) => return out,
    };

    for node in document.select(&selector) {
        if let Some(href) = node.value().attr("href") {
            if href.to_ascii_lowercase().contains("docx") {
                out.insert(href.to_string());
            }
        }
    }

    out
}

fn extract_school_links(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let document = Html::parse_document(html);
    let selector = match Selector::parse("div.PanelsSchools span.wikilink a[href]") {
        Ok(v) => v,
        Err(_) => return out,
    };

    for node in document.select(&selector) {
        let href = node.value().attr("href").unwrap_or_default().to_string();
        if href.trim().is_empty() {
            continue;
        }
        let name = node.text().collect::<String>().trim().to_string();
        out.push((name, href));
    }

    out
}

fn extract_team_page_links(html: &str, base_url: &str) -> Vec<String> {
    let mut out = Vec::new();
    let document = Html::parse_document(html);
    let row_selector = match Selector::parse("table#tblTeams tr") {
        Ok(v) => v,
        Err(_) => return out,
    };
    let td_anchor_selector = match Selector::parse("td a[href]") {
        Ok(v) => v,
        Err(_) => return out,
    };

    for row in document.select(&row_selector) {
        let mut hrefs = Vec::new();
        for anchor in row.select(&td_anchor_selector) {
            if let Some(href) = anchor.value().attr("href") {
                hrefs.push(absolute_url(base_url, href));
            }
        }
        if hrefs.len() >= 2 {
            out.push(hrefs[0].clone());
            out.push(hrefs[1].clone());
        }
    }

    out
}

fn parse_filename_from_url(url: &str) -> Option<String> {
    let part = url.rsplit('/').next()?.split('?').next()?.trim();
    if part.is_empty() {
        return None;
    }
    Some(part.to_string())
}

async fn upload_then_index(client: &reqwest::Client, backend_api_url: &str, file_path: &Path) -> Result<(), String> {
    let filename = file_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "invalid filename".to_string())?
        .to_string();

    let bytes = fs::read(file_path).map_err(|error| format!("read failed for {filename}: {error}"))?;
    let part = Part::bytes(bytes)
        .file_name(filename.clone())
        .mime_str("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        .map_err(|error| format!("mime failed for {filename}: {error}"))?;

    let upload_url = format!("{}/upload-docx", backend_api_url.trim_end_matches('/'));
    let form = Form::new().part("file", part).text("parse", "false");
    let upload_response = client
        .post(upload_url)
        .multipart(form)
        .send()
        .await
        .map_err(|error| format!("upload request failed for {filename}: {error}"))?;

    if !upload_response.status().is_success() {
        let status = upload_response.status();
        let body = upload_response
            .text()
            .await
            .unwrap_or_else(|_| "<no body>".to_string());
        return Err(format!("upload failed for {filename}: {status} {body}"));
    }

    let upload_payload: Value = upload_response
        .json()
        .await
        .map_err(|error| format!("invalid upload JSON for {filename}: {error}"))?;
    let stored_filename = upload_payload
        .get("filename")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&filename)
        .to_string();

    let index_url = format!("{}/index-document", backend_api_url.trim_end_matches('/'));
    let index_response = client
        .post(index_url)
        .json(&json!({ "filename": stored_filename }))
        .send()
        .await
        .map_err(|error| format!("index request failed for {filename}: {error}"))?;

    if !index_response.status().is_success() {
        let status = index_response.status();
        let body = index_response
            .text()
            .await
            .unwrap_or_else(|_| "<no body>".to_string());
        return Err(format!("index failed for {filename}: {status} {body}"));
    }

    Ok(())
}

async fn indexed_filename_set(client: &reqwest::Client, backend_api_url: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let url = format!("{}/documents", backend_api_url.trim_end_matches('/'));
    let response = match client.get(url).send().await {
        Ok(v) => v,
        Err(_) => return out,
    };
    let payload: Value = match response.json().await {
        Ok(v) => v,
        Err(_) => return out,
    };

    if let Some(items) = payload.get("documents").and_then(Value::as_array) {
        for item in items {
            let in_index = item.get("in_index").and_then(Value::as_bool).unwrap_or(false);
            if !in_index {
                continue;
            }
            if let Some(name) = item.get("filename").and_then(Value::as_str) {
                if !name.trim().is_empty() {
                    out.insert(name.to_ascii_lowercase());
                }
            }
        }
    }

    out
}

fn indexed_filename_set_direct(index_path: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    for card in load_cards(index_path) {
        if let Some(filename) = card.get("filename").and_then(Value::as_str) {
            if !filename.trim().is_empty() {
                out.insert(filename.to_ascii_lowercase());
            }
        }
    }
    out
}

async fn process_docx(config: &ScraperConfig, client: &reqwest::Client, path: &Path) -> Result<(), String> {
    match config.bridge_mode {
        BridgeMode::Direct => {
            let stored_filename = ensure_uploaded_copy(path, &config.backend_paths.local_docs_folder)?;
            index_uploaded_filename_via_backend(
                &config.backend_executable,
                &config.backend_paths,
                &stored_filename,
            )
        }
        BridgeMode::Api => upload_then_index(client, &config.backend_api_url, path).await,
    }
}

pub async fn run_scraper(config: ScraperConfig) -> Result<(), String> {
    let folder = config.tmp_folder.join(&config.division).join(&config.year);
    fs::create_dir_all(&folder).map_err(|error| format!("failed creating folder {}: {error}", folder.to_string_lossy()))?;
    let download_doc_path = folder.join("download_urls.txt");

    let client = reqwest::Client::new();

    if !(config.skip_scrape_when_urls_exist && download_doc_path.exists()) {
        let main_html = client
            .get(&config.wiki_url)
            .send()
            .await
            .map_err(|error| format!("failed requesting main page: {error}"))?
            .text()
            .await
            .map_err(|error| format!("failed reading main page: {error}"))?;

        let schools = extract_school_links(&main_html);
        let mut links = HashSet::new();

        if schools.is_empty() {
            links.extend(extract_docx_links(&main_html));
        } else {
            for (school_name, href) in schools {
                let school_url = absolute_url(&config.wiki_url, &href);
                let school_html = match client.get(&school_url).send().await {
                    Ok(resp) => match resp.text().await {
                        Ok(text) => text,
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                };

                let team_pages = extract_team_page_links(&school_html, &config.wiki_url);
                for page_url in team_pages {
                    let page_html = match client.get(&page_url).send().await {
                        Ok(resp) => match resp.text().await {
                            Ok(text) => text,
                            Err(_) => continue,
                        },
                        Err(_) => continue,
                    };
                    links.extend(extract_docx_links(&page_html));
                }

                println!("Scraped {}", school_name);
            }
        }

        let mut download_urls: HashMap<String, String> = HashMap::new();
        for url in links {
            let Some(filename) = parse_filename_from_url(&url) else {
                continue;
            };
            download_urls.insert(filename.clone(), url.clone());

            let target_path = folder.join(&filename);
            if target_path.exists() {
                continue;
            }

            let bytes = match client.get(&url).send().await {
                Ok(resp) => match resp.bytes().await {
                    Ok(bytes) => bytes,
                    Err(_) => continue,
                },
                Err(_) => continue,
            };

            if fs::write(&target_path, &bytes).is_ok() {
                println!("Downloaded {}", filename);
            }
        }

        let serialized = serde_json::to_string_pretty(&download_urls)
            .map_err(|error| format!("failed serializing download urls: {error}"))?;
        fs::write(&download_doc_path, serialized)
            .map_err(|error| format!("failed writing {}: {error}", download_doc_path.to_string_lossy()))?;
    }

    let indexed = if config.check_existing {
        match config.bridge_mode {
            BridgeMode::Direct => indexed_filename_set_direct(&config.backend_paths.local_index_path),
            BridgeMode::Api => indexed_filename_set(&client, &config.backend_api_url).await,
        }
    } else {
        HashSet::new()
    };

    let mut uploaded = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    let entries = fs::read_dir(&folder)
        .map_err(|error| format!("failed listing folder {}: {error}", folder.to_string_lossy()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !ext.eq_ignore_ascii_case("docx") {
            continue;
        }

        let filename = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();

        if config.check_existing && indexed.contains(&filename.to_ascii_lowercase()) {
            skipped += 1;
            println!("{} already in index, skipping", filename);
            continue;
        }

        match process_docx(&config, &client, &path).await {
            Ok(_) => {
                uploaded += 1;
                println!("{} processed", filename);
            }
            Err(error) => {
                failed += 1;
                eprintln!("{} failed: {}", filename, error);
            }
        }
    }

    println!(
        "Scraper complete: uploaded={} skipped={} failed={} folder={}",
        uploaded,
        skipped,
        failed,
        folder.to_string_lossy()
    );

    if failed > 0 {
        return Err(format!("{} document(s) failed", failed));
    }

    Ok(())
}

pub fn scraper_config_with_env(
    wiki_url: String,
    division: String,
    year: String,
    tmp_folder: PathBuf,
    check_existing: bool,
    skip_scrape_when_urls_exist: bool,
) -> ScraperConfig {
    ScraperConfig {
        wiki_url,
        division,
        year,
        tmp_folder,
        backend_api_url: std::env::var("BACKEND_API_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:5002".to_string()),
        bridge_mode: BridgeMode::from_env(),
        backend_executable: backend_executable_from_env(),
        backend_paths: BackendPaths::from_env(),
        check_existing,
        skip_scrape_when_urls_exist,
    }
}