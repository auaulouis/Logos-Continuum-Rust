pub mod app_config;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use app_config::BackendPaths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMode {
	Direct,
	Api,
}

impl BridgeMode {
	pub fn from_env() -> Self {
		match env::var("BACKEND_BRIDGE_MODE")
			.unwrap_or_else(|_| "direct".to_string())
			.trim()
			.to_ascii_lowercase()
			.as_str()
		{
			"api" | "http" => Self::Api,
			_ => Self::Direct,
		}
	}
}

pub fn backend_executable_from_env() -> PathBuf {
	if let Ok(raw) = env::var("BACKEND_EXECUTABLE") {
		let trimmed = raw.trim();
		if !trimmed.is_empty() {
			return PathBuf::from(trimmed);
		}
	}

	if Path::new("./target/release/logos-backend").exists() {
		return PathBuf::from("./target/release/logos-backend");
	}

	PathBuf::from("./target/debug/logos-backend")
}

pub fn ensure_uploaded_copy(source_file: &Path, local_docs_folder: &Path) -> Result<String, String> {
	if !source_file.is_file() {
		return Err(format!("source file not found: {}", source_file.to_string_lossy()));
	}

	let raw_name = source_file
		.file_name()
		.and_then(|value| value.to_str())
		.ok_or_else(|| "invalid source filename".to_string())?
		.to_string();

	if !raw_name.to_ascii_lowercase().ends_with(".docx") {
		return Err("Only .docx files are supported".to_string());
	}

	let safe_name = sanitize_filename(&raw_name);
	let upload_dir = local_docs_folder.join("uploaded_docs");
	fs::create_dir_all(&upload_dir)
		.map_err(|error| format!("failed creating upload directory: {error}"))?;

	let target_path = dedupe_target_path(&upload_dir, &safe_name);
	fs::copy(source_file, &target_path)
		.map_err(|error| format!("failed copying file to uploaded_docs: {error}"))?;

	target_path
		.file_name()
		.and_then(|value| value.to_str())
		.map(|value| value.to_string())
		.ok_or_else(|| "failed resolving stored filename".to_string())
}

pub fn index_uploaded_filename_via_backend(
	backend_executable: &Path,
	backend_paths: &BackendPaths,
	filename: &str,
) -> Result<(), String> {
	let normalized = filename.trim();
	if normalized.is_empty() {
		return Err("filename is required".to_string());
	}

	let output = Command::new(backend_executable)
		.env("INDEX_ONE_FILENAME", normalized)
		.env("LOCAL_DOCS_FOLDER", backend_paths.local_docs_folder.to_string_lossy().to_string())
		.env("LOCAL_INDEX_PATH", backend_paths.local_index_path.to_string_lossy().to_string())
		.env("INDEX_CACHE_PATH", backend_paths.index_cache_path.to_string_lossy().to_string())
		.env("PARSER_SETTINGS_PATH", backend_paths.parser_settings_path.to_string_lossy().to_string())
		.env("PARSER_EVENTS_PATH", backend_paths.parser_events_path.to_string_lossy().to_string())
		.output()
		.map_err(|error| format!("failed launching backend executable: {error}"))?;

	if output.status.success() {
		return Ok(());
	}

	let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
	let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
	let detail = if !stderr.is_empty() {
		stderr
	} else if !stdout.is_empty() {
		stdout
	} else {
		format!("exit status {}", output.status)
	};

	Err(format!("backend indexing failed for {}: {}", normalized, detail))
}

fn dedupe_target_path(upload_dir: &Path, safe_name: &str) -> PathBuf {
	let candidate = upload_dir.join(safe_name);
	if !candidate.exists() {
		return candidate;
	}

	let path = Path::new(safe_name);
	let stem = path
		.file_stem()
		.and_then(|value| value.to_str())
		.unwrap_or("document");
	let extension = path
		.extension()
		.and_then(|value| value.to_str())
		.unwrap_or("docx");

	let mut suffix = 1_u32;
	loop {
		let deduped = upload_dir.join(format!("{}-{}.{}", stem, suffix, extension));
		if !deduped.exists() {
			return deduped;
		}
		suffix += 1;
	}
}

fn sanitize_filename(value: &str) -> String {
	let mut out = String::new();
	for ch in value.chars() {
		if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ') {
			out.push(ch);
		} else {
			out.push('_');
		}
	}

	let trimmed = out.trim().trim_matches('.').trim();
	if trimmed.is_empty() {
		"upload.docx".to_string()
	} else {
		trimmed.to_string()
	}
}
