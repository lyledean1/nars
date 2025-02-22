use crate::editor::{run, Editor};
use crate::models::ollama::OllamaClient;
use crate::models::Predictor;
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
    let (mut editor, prediction_tx) = Editor::new(filename.clone().unwrap_or(".rs".to_string()));
    if let Some(path) = filename {
        editor.load_file(path)?;
    }
    let predictor = Arc::new(Predictor::new(client, prediction_tx));
    run(editor, predictor).await
}
