use std::env;
use std::path::PathBuf;

#[path = "../scraper_common.rs"]
mod scraper_common;

use scraper_common::{run_scraper, scraper_config_with_env};

#[tokio::main]
async fn main() {
    let config = scraper_config_with_env(
        env::var("SCRAPER_WIKI_URL")
            .unwrap_or_else(|_| "https://opencaselist.paperlessdebate.com".to_string()),
        env::var("SCRAPER_DIVISION").unwrap_or_else(|_| "college".to_string()),
        env::var("SCRAPER_YEAR").unwrap_or_else(|_| "21-22".to_string()),
        PathBuf::from(env::var("SCRAPER_TMP_FOLDER").unwrap_or_else(|_| "./tmp".to_string())),
        env::var("SCRAPER_CHECK_EXISTING")
            .ok()
            .map(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(true),
        env::var("SCRAPER_SKIP_SCRAPE_WHEN_URLS_EXIST")
            .ok()
            .map(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false),
    );

    if let Err(error) = run_scraper(config).await {
        eprintln!("scraper failed: {}", error);
        std::process::exit(1);
    }
}