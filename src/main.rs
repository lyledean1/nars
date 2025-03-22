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
    let mut model = "qwen2.5-coder:7b".to_string();
    if args.len() >= 2 {
        model = args.get(2).cloned().unwrap_or(model.to_string());
    }
    let predictor = Arc::new(Predictor::new(client, prediction_tx, model));
    run(editor, predictor).await
}
