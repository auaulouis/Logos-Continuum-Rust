use std::cmp::Ordering;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use logos_backend::app_config::BackendPaths;
use logos_backend::{
    backend_executable_from_env, ensure_uploaded_copy, index_uploaded_filename_via_backend, BridgeMode,
};
use reqwest::multipart::{Form, Part};
use serde_json::{json, Value};
use walkdir::WalkDir;

fn is_docx(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("docx"))
        .unwrap_or(false)
}

fn is_under(path: &Path, folder: &Path) -> bool {
    let p = match path.canonicalize() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let f = match folder.canonicalize() {
        Ok(v) => v,
        Err(_) => return false,
    };
    p.starts_with(f)
}

fn move_to_done(filepath: &Path, local_folder: &Path, done_folder: &Path) -> Result<PathBuf, String> {
    let rel = filepath
        .strip_prefix(local_folder)
        .map_err(|error| format!("failed computing relative path: {error}"))?;
    let target = done_folder.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("failed creating done folder: {error}"))?;
    }

    if !target.exists() {
        fs::rename(filepath, &target).map_err(|error| format!("failed moving file: {error}"))?;
        return Ok(target);
    }

    let stem = target
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let ext = target.extension().and_then(|value| value.to_str()).unwrap_or("docx");

    let mut suffix = 1usize;
    loop {
        let candidate = target.with_file_name(format!("{stem}-{suffix}.{ext}"));
        if !candidate.exists() {
            fs::rename(filepath, &candidate).map_err(|error| format!("failed moving file: {error}"))?;
            return Ok(candidate);
        }
        suffix += 1;
    }
}

async fn upload_then_index(
    client: &reqwest::Client,
    backend_api_url: &str,
    source_file: &Path,
) -> Result<String, String> {
    let filename = source_file
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "invalid filename".to_string())?
        .to_string();

    let bytes = fs::read(source_file).map_err(|error| format!("read failed for {filename}: {error}"))?;
    let file_part = Part::bytes(bytes)
        .file_name(filename.clone())
        .mime_str("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        .map_err(|error| format!("mime build failed: {error}"))?;
    let form = Form::new().part("file", file_part).text("parse", "false");

    let upload_url = format!("{}/upload-docx", backend_api_url.trim_end_matches('/'));
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

    let upload_json: Value = upload_response
        .json()
        .await
        .map_err(|error| format!("invalid upload response JSON for {filename}: {error}"))?;

    let stored_filename = upload_json
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

    Ok(filename)
}

fn index_direct(source_file: &Path, backend_executable: &Path, backend_paths: &BackendPaths) -> Result<String, String> {
    let stored_filename = ensure_uploaded_copy(source_file, &backend_paths.local_docs_folder)?;
    index_uploaded_filename_via_backend(backend_executable, backend_paths, &stored_filename)?;
    Ok(stored_filename)
}

#[tokio::main]
async fn main() {
    let local_folder = PathBuf::from(env::var("LOCAL_DOCS_FOLDER").unwrap_or_else(|_| "./local_docs".to_string()));
    let done_subdir = env::var("DONE_SUBDIR").unwrap_or_else(|_| "done".to_string());
    let done_folder = local_folder.join(done_subdir);
    let backend_api_url = env::var("BACKEND_API_URL").unwrap_or_else(|_| "http://127.0.0.1:5002".to_string());
    let backend_paths = BackendPaths::from_env();
    let bridge_mode = BridgeMode::from_env();
    let backend_executable = backend_executable_from_env();
    let sort_mode = env::var("LOCAL_PARSER_SORT").unwrap_or_else(|_| "size_asc".to_string());
    let progress_every = env::var("LOCAL_PARSER_PROGRESS_EVERY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(250)
        .max(1);
    let upload_folder = backend_paths.local_docs_folder.join("uploaded_docs");

    let mut files: Vec<PathBuf> = WalkDir::new(&local_folder)
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| path.is_file() && is_docx(path))
        .filter(|path| {
            path.file_name()
                .and_then(|v| v.to_str())
                .map(|name| !name.starts_with("~$"))
                .unwrap_or(false)
        })
        .filter(|path| !is_under(path, &done_folder))
        .filter(|path| !is_under(path, &upload_folder))
        .collect();

    if files.is_empty() {
        println!(
            "⚠️ No new files found in {} (excluding {})",
            local_folder.to_string_lossy(),
            done_folder.to_string_lossy()
        );
        return;
    }

    match sort_mode.trim().to_ascii_lowercase().as_str() {
        "size_desc" => {
            files.sort_by(|a, b| {
                let a_len = fs::metadata(a).map(|m| m.len()).unwrap_or(0);
                let b_len = fs::metadata(b).map(|m| m.len()).unwrap_or(0);
                b_len.cmp(&a_len)
            });
        }
        "name" => files.sort_by(|a, b| {
            let a_name = a
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let b_name = b
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            a_name.cmp(&b_name)
        }),
        _ => {
            files.sort_by(|a, b| {
                let a_len = fs::metadata(a).map(|m| m.len()).unwrap_or(0);
                let b_len = fs::metadata(b).map(|m| m.len()).unwrap_or(0);
                match a_len.cmp(&b_len) {
                    Ordering::Equal => a.cmp(b),
                    other => other,
                }
            });
        }
    }

    println!(
        "🔎 Found {} files. Starting Rust local parser migration... (sort={})",
        files.len(),
        sort_mode
    );
    println!("⚙️  Done folder: {}", done_folder.to_string_lossy());
    match bridge_mode {
        BridgeMode::Direct => {
            println!("⚙️  Bridge mode: direct");
            println!("⚙️  Backend executable: {}", backend_executable.to_string_lossy());
            println!("⚙️  Backend local_docs: {}", backend_paths.local_docs_folder.to_string_lossy());
        }
        BridgeMode::Api => {
            println!("⚙️  Bridge mode: api");
            println!("⚙️  Backend API: {}", backend_api_url);
        }
    }

    let client = reqwest::Client::new();
    let started = Instant::now();
    let mut processed = 0usize;
    let mut failed = 0usize;

    for (index, path) in files.iter().enumerate() {
        let display_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<unknown>");

        let parse_result = match bridge_mode {
            BridgeMode::Direct => index_direct(path, &backend_executable, &backend_paths),
            BridgeMode::Api => upload_then_index(&client, &backend_api_url, path).await,
        };

        match parse_result {
            Ok(_) => {
                if let Err(error) = move_to_done(path, &local_folder, &done_folder) {
                    failed += 1;
                    eprintln!("❌ Parsed but failed moving {}: {}", display_name, error);
                } else {
                    processed += 1;
                    println!("✅ Parsed and moved {}", display_name);
                }
            }
            Err(error) => {
                failed += 1;
                eprintln!("❌ Failed {}: {}", display_name, error);
            }
        }

        let current = index + 1;
        if current % progress_every == 0 || current == files.len() {
            let elapsed = started.elapsed().as_secs_f64();
            println!(
                "📊 Progress: {}/{} | success={} | failed={} | elapsed={:.1}s",
                current,
                files.len(),
                processed,
                failed,
                elapsed
            );
        }
    }

    println!(
        "⏱️ Completed {}/{} files with {} failures in {:.1}s",
        processed,
        files.len(),
        failed,
        started.elapsed().as_secs_f64()
    );

    if failed > 0 {
        std::process::exit(1);
    }
}