//! Grok Client — streaming chat completions via the xAI API.
//!
//! The xAI API is OpenAI-compatible (`/v1/chat/completions`).  This module
//! wraps `reqwest` to send chat requests and stream back SSE delta chunks,
//! emitting each token to the Tauri frontend in real time.

use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

const XAI_API_URL: &str = "https://api.x.ai/v1/chat/completions";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single message in the chat history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Request body sent to the xAI completions endpoint.
#[derive(Debug, Serialize)]
struct CompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    temperature: f32,
}

/// Top-level SSE chunk returned when streaming.
#[derive(Debug, Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: Delta,
}

#[derive(Debug, Deserialize)]
struct Delta {
    content: Option<String>,
}

// ---------------------------------------------------------------------------
// GrokClient
// ---------------------------------------------------------------------------

/// Manages communication with the xAI Grok API.
pub struct GrokClient {
    http: Client,
    api_key: String,
    model: String,
}

impl GrokClient {
    /// Create a new client.  `api_key` is read from the `XAI_API_KEY`
    /// environment variable at startup.
    pub fn new(api_key: String) -> Self {
        Self {
            http: Client::new(),
            api_key,
            model: "grok-3-fast".to_string(),
        }
    }

    /// Check whether an API key was provided.
    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    /// Send a non-streaming completion and return the full response text.
    pub async fn complete(&self, messages: Vec<ChatMessage>) -> Result<String, String> {
        let body = CompletionRequest {
            model: self.model.clone(),
            messages,
            stream: false,
            temperature: 0.3,
        };

        let resp = self
            .http
            .post(XAI_API_URL)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;

        if !status.is_success() {
            return Err(format!("xAI API error ({status}): {text}"));
        }

        // Parse and extract the assistant message content.
        let val: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| e.to_string())?;
        val["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "unexpected response shape".to_string())
    }

    /// Stream a completion, emitting `grok-token` events for each delta.
    /// When the stream finishes, emits `grok-done`.
    pub async fn stream_complete(
        &self,
        app: &AppHandle,
        messages: Vec<ChatMessage>,
    ) -> Result<(), String> {
        let body = CompletionRequest {
            model: self.model.clone(),
            messages,
            stream: true,
            temperature: 0.3,
        };

        let resp = self
            .http
            .post(XAI_API_URL)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("xAI API error ({status}): {text}"));
        }

        // Read the SSE byte stream.
        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // SSE frames are separated by double newlines.
            while let Some(pos) = buffer.find("\n\n") {
                let frame = buffer[..pos].to_string();
                buffer = buffer[pos + 2..].to_string();

                for line in frame.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        if data.trim() == "[DONE]" {
                            let _ = app.emit("grok-done", ());
                            return Ok(());
                        }
                        if let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) {
                            for choice in chunk.choices {
                                if let Some(content) = choice.delta.content {
                                    let _ = app.emit("grok-token", content);
                                }
                            }
                        }
                    }
                }
            }
        }

        let _ = app.emit("grok-done", ());
        Ok(())
    }
}
