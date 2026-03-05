use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct BackendPaths {
    pub local_docs_folder: PathBuf,
    pub local_index_path: PathBuf,
    pub index_cache_path: PathBuf,
    pub parser_settings_path: PathBuf,
    pub parser_events_path: PathBuf,
}

impl BackendPaths {
    pub fn from_local_docs_folder(local_docs_folder: PathBuf) -> Self {
        Self {
            local_index_path: local_docs_folder.join("cards_index.json"),
            index_cache_path: local_docs_folder.join("cards_cache.bin"),
            parser_settings_path: local_docs_folder.join("parser_settings.json"),
            parser_events_path: local_docs_folder.join("parser_events.jsonl"),
            local_docs_folder,
        }
    }

    pub fn from_env() -> Self {
        let local_docs_folder = PathBuf::from(
            env::var("LOCAL_DOCS_FOLDER").unwrap_or_else(|_| "./local_docs".to_string()),
        );

        let mut paths = Self::from_local_docs_folder(local_docs_folder);

        if let Ok(value) = env::var("LOCAL_INDEX_PATH") {
            paths.local_index_path = PathBuf::from(value);
        }

        if let Ok(value) = env::var("INDEX_CACHE_PATH") {
            paths.index_cache_path = PathBuf::from(value);
        }

        if let Ok(value) = env::var("PARSER_SETTINGS_PATH") {
            paths.parser_settings_path = PathBuf::from(value);
        }

        if let Ok(value) = env::var("PARSER_EVENTS_PATH") {
            paths.parser_events_path = PathBuf::from(value);
        }

        paths
    }
}
