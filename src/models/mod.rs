use std::sync::Arc;
use tokio::sync::mpsc;
use crate::models::ollama::OllamaClient;
use parser::{parse_code_output, ParsedCode};
use futures_util::StreamExt;
use tokio::task;
use crate::logger::log_to_file;

pub mod ollama;
pub mod parser;

async fn stream_prediction(
    client: Arc<OllamaClient>,
    prediction_tx: mpsc::Sender<String>,
    line: String,
) -> anyhow::Result<String> {
    let prompt = format!("Complete the code on this line, returning only the raw code without any formatting, comments, or extra text. Example input: 'let x = '  Example output: 'let x = Some(42);'. Here is the code {}", line);
    log_to_file(&prompt);
    let mut stream = client
        .stream_generate("qwen2.5-coder:7b", prompt.as_str())
        .await?;
    let mut pred = "".to_string();
    let mut output = ParsedCode {
        code: "".to_string(),
    };

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(text) => {
                pred = format!("{}{}", pred, text);
                log_to_file(format!("Next chunk {}", pred).as_str());
                // refactor as this is not needed or return this?
                output = parse_code_output(&pred)?;
                match prediction_tx.send(pred.to_string()).await {
                    Ok(_) => {
                        log_to_file(format!("Send pred to channel {}", pred).as_str());
                    }
                    Err(e) => {
                        eprintln!("Failed to send prediction: {}", e);
                    }
                }
            }
            Err(e) => eprintln!("Error: {}", e),
        }
    }
    log_to_file(&pred);
    Ok(output.code)
}

pub async fn stream_prediction_background(
    client: Arc<OllamaClient>,
    content: String,
    prediction_tx: mpsc::Sender<String>,
) {
    task::spawn(async move {
        if let Err(e) = stream_prediction(client, prediction_tx, content).await {
            log_to_file(format!("Prediction error: {}", e.to_string().as_str()).as_str());
        }
    });
}