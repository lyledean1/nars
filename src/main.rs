use crate::editor::run_editor;
use crate::models::ollama::OllamaClient;
use anyhow::Result;
use std::env;
use std::sync::Arc;

mod editor;
mod logger;
mod models;

#[tokio::main]
async fn main() -> Result<()> {
    let client = Arc::new(OllamaClient::new());
    let args: Vec<String> = env::args().collect();
    let filename = args.get(1).cloned();
    run_editor(client, filename).await
}
