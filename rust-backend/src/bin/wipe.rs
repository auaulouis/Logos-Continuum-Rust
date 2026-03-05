use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let local_docs = PathBuf::from(env::var("LOCAL_DOCS_FOLDER").unwrap_or_else(|_| "./local_docs".to_string()));
    let index_path = PathBuf::from(
        env::var("LOCAL_INDEX_PATH")
            .unwrap_or_else(|_| local_docs.join("cards_index.json").to_string_lossy().to_string()),
    );

    if let Some(parent) = index_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    println!("🗑️  Clearing local card index...");
    match fs::write(&index_path, "") {
        Ok(_) => println!("✅ Local index cleared! ({})", index_path.to_string_lossy()),
        Err(error) => {
            eprintln!(
                "❌ Failed to clear local index at {}: {}",
                index_path.to_string_lossy(),
                error
            );
            std::process::exit(1);
        }
    }
}