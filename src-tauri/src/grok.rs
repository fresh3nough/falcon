//! Grok Client — streaming chat completions via the xAI API.
//!
//! The xAI API is OpenAI-compatible (`/v1/chat/completions`).  This module
//! wraps `reqwest` to send chat requests and stream back SSE delta chunks,
//! emitting each token to the Tauri frontend in real time.

use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tauri::{AppHandle, Emitter};

const XAI_API_URL: &str = "https://api.x.ai/v1/chat/completions";

/// Vision-capable model for multimodal queries.
const XAI_VISION_MODEL: &str = "grok-2-vision-1212";

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
// Tool-calling types (agent mode)
// ---------------------------------------------------------------------------

/// Tool definition sent to the Grok API for function calling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDef,
}

/// Schema for a callable function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A tool call returned by the model in a completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub function: FunctionCall,
}

/// The function name and serialized arguments of a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Accumulated response from a tool-calling completion.
pub struct AgentResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

/// Request body for tool-calling completions (uses raw JSON messages).
#[derive(Debug, Serialize)]
struct ToolCompletionRequest {
    model: String,
    messages: Vec<serde_json::Value>,
    stream: bool,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDef>>,
}

/// Streaming chunk that may contain tool call deltas.
#[derive(Debug, Deserialize)]
struct ToolStreamChunk {
    choices: Vec<ToolStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct ToolStreamChoice {
    delta: ToolDelta,
}

#[derive(Debug, Deserialize)]
struct ToolDelta {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
    index: usize,
    id: Option<String>,
    function: Option<FunctionCallDelta>,
}

#[derive(Debug, Deserialize)]
struct FunctionCallDelta {
    name: Option<String>,
    arguments: Option<String>,
}

// ---------------------------------------------------------------------------
// GrokClient
// ---------------------------------------------------------------------------

/// Manages communication with the xAI Grok API.
#[derive(Clone)]
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

    /// Stream a multimodal (vision) chat completion.
    ///
    /// `image_urls` are base64 data URLs.  The request uses the vision model
    /// and the standard `grok-token` / `grok-done` event pair.
    pub async fn stream_vision_chat(
        &self,
        app: &AppHandle,
        user_text: &str,
        image_urls: Vec<String>,
        system_prompt: &str,
    ) -> Result<(), String> {
        // Build multimodal content array.
        let mut content_parts: Vec<serde_json::Value> = Vec::new();
        content_parts.push(serde_json::json!({
            "type": "text",
            "text": user_text,
        }));
        for url in &image_urls {
            content_parts.push(serde_json::json!({
                "type": "image_url",
                "image_url": { "url": url },
            }));
        }

        let messages = serde_json::json!([
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": content_parts },
        ]);

        let body = serde_json::json!({
            "model": XAI_VISION_MODEL,
            "messages": messages,
            "stream": true,
            "temperature": 0.3,
        });

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
            return Err(format!("xAI Vision API error ({status}): {text}"));
        }

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

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

    /// Stream a tool-calling completion.  Emits thinking tokens via
    /// `thinking_event` and returns the accumulated content + tool calls.
    pub async fn stream_complete_with_tools(
        &self,
        app: &AppHandle,
        messages: Vec<serde_json::Value>,
        tools: Vec<ToolDef>,
        thinking_event: &str,
    ) -> Result<AgentResponse, String> {
        let body = ToolCompletionRequest {
            model: self.model.clone(),
            messages,
            stream: true,
            temperature: 0.3,
            tools: if tools.is_empty() { None } else { Some(tools) },
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

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        let mut content = String::new();
        // Accumulate tool calls by stream index: (id, name, arguments).
        let mut tc_map: BTreeMap<usize, (String, String, String)> = BTreeMap::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buffer.find("\n\n") {
                let frame = buffer[..pos].to_string();
                buffer = buffer[pos + 2..].to_string();

                for line in frame.lines() {
                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };
                    if data.trim() == "[DONE]" {
                        let tool_calls = tc_map
                            .into_values()
                            .map(|(id, name, args)| ToolCall {
                                id,
                                function: FunctionCall {
                                    name,
                                    arguments: args,
                                },
                            })
                            .collect();
                        return Ok(AgentResponse { content, tool_calls });
                    }
                    if let Ok(parsed) =
                        serde_json::from_str::<ToolStreamChunk>(data)
                    {
                        for choice in parsed.choices {
                            if let Some(c) = choice.delta.content {
                                content.push_str(&c);
                                let _ = app.emit(thinking_event, &c);
                            }
                            if let Some(deltas) = choice.delta.tool_calls {
                                for d in deltas {
                                    let entry = tc_map
                                        .entry(d.index)
                                        .or_insert_with(|| {
                                            (String::new(), String::new(), String::new())
                                        });
                                    if let Some(id) = d.id {
                                        entry.0 = id;
                                    }
                                    if let Some(f) = d.function {
                                        if let Some(name) = f.name {
                                            entry.1 = name;
                                        }
                                        if let Some(args) = f.arguments {
                                            entry.2.push_str(&args);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Stream ended without explicit [DONE] marker.
        let tool_calls = tc_map
            .into_values()
            .map(|(id, name, args)| ToolCall {
                id,
                function: FunctionCall {
                    name,
                    arguments: args,
                },
            })
            .collect();
        Ok(AgentResponse { content, tool_calls })
    }
}
