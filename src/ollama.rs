use anyhow::Result;
use futures_util::StreamExt;
use futures_util::{Stream, TryStreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

const OLLAMA_BASE_URL: &str = "http://localhost:11434/api";

#[derive(Serialize)]
struct GenerateRequest {
    model: String,
    prompt: String,
    stream: bool,
}

#[derive(Deserialize)]
struct GenerateResponse {
    model: String,
    response: String,
    done: bool,
}

#[derive(Clone)]
pub struct OllamaClient {
    client: Client,
}

impl OllamaClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn generate(&self, model: &str, prompt: &str) -> Result<String> {
        let request = GenerateRequest {
            model: model.to_string(),
            prompt: prompt.to_string(),
            stream: false,
        };

        let response = self
            .client
            .post(format!("{}/generate", OLLAMA_BASE_URL))
            .json(&request)
            .send()
            .await?;

        let generation: GenerateResponse = response.json().await?;
        Ok(generation.response)
    }

    pub async fn stream_generate(
        &self,
        model: &str,
        prompt: &str,
    ) -> Result<impl Stream<Item = Result<String>>> {
        let request = GenerateRequest {
            model: model.to_string(),
            prompt: prompt.to_string(),
            stream: true,
        };

        let response = self
            .client
            .post(format!("{}/generate", OLLAMA_BASE_URL))
            .json(&request)
            .send()
            .await?;

        Ok(response
            .bytes_stream()
            .map_err(|e| anyhow::anyhow!("Stream error: {}", e))
            .map(|chunk| -> Result<String> {
                let bytes = chunk?;
                let response: GenerateResponse = serde_json::from_slice(&bytes)?;
                Ok(response.response)
            }))
    }
}
