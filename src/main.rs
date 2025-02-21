use std::sync::Arc;
use anyhow::Result;
use crate::models::ollama::OllamaClient;
use std::{env};
use crate::editor::run_editor;

mod models;
mod logger;
mod editor;

#[tokio::main]
async fn main() -> Result<()> {
    let client = Arc::new(OllamaClient::new());
    let args: Vec<String> = env::args().collect();
    let filename = args.get(1).cloned();
    run_editor(client, filename).await
}
