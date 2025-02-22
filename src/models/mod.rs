use crate::logger::log_to_file;
use crate::models::ollama::OllamaClient;
use anyhow::Result;
use futures_util::StreamExt;
use parser::{parse_code_output, ParsedCode};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task;

pub mod ollama;
pub mod parser;

pub struct Predictor {
    client: Arc<OllamaClient>,
    prediction_tx: mpsc::Sender<String>,
}

impl Predictor {
    pub fn new(client: Arc<OllamaClient>, prediction_tx: mpsc::Sender<String>) -> Self {
        Predictor {
            client,
            prediction_tx,
        }
    }

    async fn stream_prediction(&self, line: String) -> Result<String> {
        let prompt = format!("Complete the code on this line, returning only the raw code without any formatting, comments, or extra text. Example input: 'let x = '  Example output: 'let x = Some(42);'. Here is the code {}", line);
        log_to_file(&prompt);
        let mut stream = self
            .client
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
                    match self.prediction_tx.send(pred.to_string()).await {
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

    pub fn stream_prediction_background(self: Arc<Self>, content: String) {
        let prediction_handler = self.clone();
        task::spawn(async move {
            if let Err(e) = prediction_handler.stream_prediction(content).await {
                log_to_file(format!("Prediction error: {}", e).as_str());
            }
        });
    }
}
