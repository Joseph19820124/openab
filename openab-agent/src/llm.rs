use anyhow::{anyhow, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::pin::Pin;

/// A message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

/// A content block within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
        /// Opaque token returned by Gemini 3.x for functionCall parts that must
        /// be echoed back on subsequent requests in the same turn. `None` for
        /// other providers and for Gemini <= 2.5.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// Tool definition sent to the LLM.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Events streamed back from the LLM.
#[derive(Debug, Clone)]
pub enum LlmEvent {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: Value,
        /// See [`ContentBlock::ToolUse::thought_signature`].
        thought_signature: Option<String>,
    },
    Stop,
    #[allow(dead_code)]
    Error(String),
}

/// Trait for LLM providers.
pub trait LlmProvider: Send + Sync {
    fn chat<'a>(
        &'a self,
        system: &'a str,
        messages: &'a [Message],
        tools: &'a [ToolDef],
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<LlmEvent>>> + Send + 'a>>;
}

/// Anthropic Claude provider.
pub struct AnthropicProvider {
    api_key: String,
    model: String,
    #[allow(dead_code)]
    max_tokens: u32,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;
        if api_key.is_empty() {
            return Err("ANTHROPIC_API_KEY is empty".to_string());
        }
        Ok(Self {
            api_key,
            model: std::env::var("OPENAB_AGENT_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string()),
            max_tokens: std::env::var("OPENAB_AGENT_MAX_TOKENS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8192),
            client: reqwest::Client::new(),
        })
    }

    fn build_request_body(&self, system: &str, messages: &[Message], tools: &[ToolDef]) -> Value {
        let msgs: Vec<Value> =
            messages
                .iter()
                .map(|m| {
                    let content: Vec<Value> = m
                    .content
                    .iter()
                    .map(|b| match b {
                        ContentBlock::Text { text } => json!({ "type": "text", "text": text }),
                        ContentBlock::ToolUse { id, name, input, .. } => {
                            json!({ "type": "tool_use", "id": id, "name": name, "input": input })
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } => {
                            let mut v = json!({
                                "type": "tool_result",
                                "tool_use_id": tool_use_id,
                                "content": content
                            });
                            if let Some(true) = is_error {
                                v["is_error"] = json!(true);
                            }
                            v
                        }
                    })
                    .collect();
                    json!({ "role": &m.role, "content": content })
                })
                .collect();

        let mut body = json!({
            "model": &self.model,
            "max_tokens": self.max_tokens,
            "messages": msgs,
            "system": system,
        });

        if !tools.is_empty() {
            let tool_defs: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": &t.name,
                        "description": &t.description,
                        "input_schema": &t.input_schema
                    })
                })
                .collect();
            body["tools"] = json!(tool_defs);
        }

        body
    }
}

impl LlmProvider for AnthropicProvider {
    fn chat<'a>(
        &'a self,
        system: &'a str,
        messages: &'a [Message],
        tools: &'a [ToolDef],
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<LlmEvent>>> + Send + 'a>> {
        Box::pin(async move {
            let body = self.build_request_body(system, messages, tools);
            let max_retries = 3u32;

            for attempt in 0..=max_retries {
                let resp = self
                    .client
                    .post("https://api.anthropic.com/v1/messages")
                    .header("x-api-key", &self.api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| anyhow!("HTTP request failed: {e}"))?;

                let status = resp.status();

                // Retry on 429 (rate limit) or 529 (overloaded)
                if (status.as_u16() == 429 || status.as_u16() == 529) && attempt < max_retries {
                    let delay = std::time::Duration::from_millis(1000 * 2u64.pow(attempt));
                    tokio::time::sleep(delay).await;
                    continue;
                }

                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(anyhow!("Anthropic API error {status}: {text}"));
                }

                let response: Value = resp
                    .json()
                    .await
                    .map_err(|e| anyhow!("Failed to parse response: {e}"))?;

                return parse_anthropic_response(&response);
            }

            Err(anyhow!("Anthropic API: max retries exceeded"))
        })
    }
}

fn parse_anthropic_response(response: &Value) -> Result<Vec<LlmEvent>> {
    let mut events = Vec::new();

    let content = response
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| anyhow!("missing content in response"))?;

    for block in content {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    events.push(LlmEvent::Text(text.to_string()));
                }
            }
            Some("tool_use") => {
                let id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let input = block.get("input").cloned().unwrap_or(json!({}));
                events.push(LlmEvent::ToolUse {
                    id,
                    name,
                    input,
                    thought_signature: None,
                });
            }
            _ => {}
        }
    }

    let stop_reason = response
        .get("stop_reason")
        .and_then(|s| s.as_str())
        .unwrap_or("end_turn");

    if stop_reason != "tool_use" {
        events.push(LlmEvent::Stop);
    }

    Ok(events)
}

// === OpenAI-compatible Provider (for Codex subscription via OAuth) ===

pub struct OpenAiProvider {
    base_url: String,
    model: String,
    #[allow(dead_code)]
    max_tokens: u32,
    client: reqwest::Client,
}

impl OpenAiProvider {
    /// Create provider using stored OAuth token from ~/.openab/agent/auth.json
    pub fn from_auth_store() -> Result<Self, String> {
        // Just verify tokens exist; actual token is fetched at call time
        crate::auth::load_tokens().map_err(|e| e.to_string())?;
        Ok(Self {
            base_url: std::env::var("OPENAB_AGENT_OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://chatgpt.com/backend-api".to_string()),
            model: std::env::var("OPENAB_AGENT_OPENAI_MODEL")
                .or_else(|_| std::env::var("OPENAB_AGENT_MODEL"))
                .unwrap_or_else(|_| "gpt-4.1-nano".to_string()),
            max_tokens: std::env::var("OPENAB_AGENT_MAX_TOKENS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8192),
            client: reqwest::Client::new(),
        })
    }
}

impl LlmProvider for OpenAiProvider {
    fn chat<'a>(
        &'a self,
        system: &'a str,
        messages: &'a [Message],
        tools: &'a [ToolDef],
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<LlmEvent>>> + Send + 'a>> {
        Box::pin(async move {
            // Build Responses API input format
            let mut oai_messages: Vec<Value> = vec![];
            for m in messages {
                if m.role == "user" {
                    // User text messages
                    let texts: Vec<&str> = m
                        .content
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlock::Text { text } = b {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect();
                    if !texts.is_empty() {
                        oai_messages.push(json!({"role": "user", "content": [{"type": "input_text", "text": texts.join("")}]}));
                    }
                    // Tool results as function_call_output
                    for b in &m.content {
                        if let ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } = b
                        {
                            oai_messages.push(json!({"type": "function_call_output", "call_id": tool_use_id, "output": content}));
                        }
                    }
                } else if m.role == "assistant" {
                    for b in &m.content {
                        match b {
                            ContentBlock::Text { text } => {
                                oai_messages.push(json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": text, "annotations": []}]}));
                            }
                            ContentBlock::ToolUse {
                                id, name, input, ..
                            } => {
                                oai_messages.push(json!({"type": "function_call", "call_id": id, "name": name, "arguments": input.to_string()}));
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Build Responses API body
            let mut body = json!({
                "model": &self.model,
                "store": false,
                "stream": true,
                "instructions": system,
                "input": oai_messages,
                "tool_choice": "auto",
                "parallel_tool_calls": true,
            });

            if !tools.is_empty() {
                let resp_tools: Vec<Value> = tools
                    .iter()
                    .map(|t| {
                        json!({
                            "type": "function",
                            "name": &t.name,
                            "description": &t.description,
                            "parameters": &t.input_schema
                        })
                    })
                    .collect();
                body["tools"] = json!(resp_tools);
            }

            let max_retries = 3u32;
            for attempt in 0..=max_retries {
                let token = crate::auth::get_valid_token().await?;
                // Extract account ID from JWT for chatgpt backend API
                let account_id = extract_account_id_from_jwt(&token);
                let mut req = self
                    .client
                    .post(format!("{}/codex/responses", self.base_url))
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Content-Type", "application/json")
                    .header("originator", "openab-agent");
                if let Some(ref aid) = account_id {
                    req = req.header("chatgpt-account-id", aid);
                }
                let resp = req
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| anyhow!("HTTP request failed: {e}"))?;

                let status = resp.status();
                if (status.as_u16() == 429 || status.as_u16() == 529) && attempt < max_retries {
                    let delay = std::time::Duration::from_millis(1000 * 2u64.pow(attempt));
                    tokio::time::sleep(delay).await;
                    continue;
                }

                // 401: token may have expired mid-request, force refresh and retry
                if status.as_u16() == 401 && attempt < max_retries {
                    let _ = crate::auth::force_refresh().await;
                    continue;
                }

                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(anyhow!("OpenAI API error {status}: {text}"));
                }

                // Parse SSE stream - collect output items from response.output_item.done events
                let text = resp
                    .text()
                    .await
                    .map_err(|e| anyhow!("Failed to read response: {e}"))?;
                let mut output_items: Vec<Value> = Vec::new();
                for line in text.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            break;
                        }
                        if let Ok(event) = serde_json::from_str::<Value>(data) {
                            let event_type =
                                event.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            if event_type == "response.output_item.done" {
                                if let Some(item) = event.get("item") {
                                    output_items.push(item.clone());
                                }
                            }
                        }
                    }
                }
                if output_items.is_empty() {
                    return Err(anyhow!(
                        "No output items in SSE stream. Raw: {}",
                        &text[..text.len().min(500)]
                    ));
                }
                let response = json!({"output": output_items});
                return parse_openai_response(&response);
            }
            Err(anyhow!("OpenAI API: max retries exceeded"))
        })
    }
}

fn extract_account_id_from_jwt(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut payload = parts[1].to_string();
    while !payload.len().is_multiple_of(4) {
        payload.push('=');
    }
    let decoded = base64::engine::general_purpose::URL_SAFE
        .decode(&payload)
        .ok()
        .or_else(|| {
            base64::engine::general_purpose::STANDARD
                .decode(&payload)
                .ok()
        })?;
    let claims: Value = serde_json::from_slice(&decoded).ok()?;
    claims["https://api.openai.com/auth"]["chatgpt_account_id"]
        .as_str()
        .map(|s| s.to_string())
}

fn parse_openai_response(response: &Value) -> Result<Vec<LlmEvent>> {
    let mut events = Vec::new();

    // Handle Responses API format (output array)
    if let Some(output) = response.get("output").and_then(|o| o.as_array()) {
        for item in output {
            match item.get("type").and_then(|t| t.as_str()) {
                Some("message") => {
                    if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                        for block in content {
                            if block.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                    events.push(LlmEvent::Text(text.to_string()));
                                }
                            }
                        }
                    }
                }
                Some("function_call") => {
                    let id = item
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let args_str = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}");
                    let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                    events.push(LlmEvent::ToolUse {
                        id,
                        name,
                        input,
                        thought_signature: None,
                    });
                }
                _ => {}
            }
        }
        events.push(LlmEvent::Stop);
        return Ok(events);
    }

    // Fallback: Chat Completions format
    let choice = response
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| anyhow!("No choices in response"))?;

    let message = choice.get("message").ok_or_else(|| anyhow!("No message"))?;

    // Text content
    if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
        if !content.is_empty() {
            events.push(LlmEvent::Text(content.to_string()));
        }
    }

    // Tool calls
    if let Some(tool_calls) = message.get("tool_calls").and_then(|t| t.as_array()) {
        for tc in tool_calls {
            let id = tc
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let args_str = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
                .unwrap_or("{}");
            let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
            events.push(LlmEvent::ToolUse {
                id,
                name,
                input,
                thought_signature: None,
            });
        }
    }

    let finish_reason = choice
        .get("finish_reason")
        .and_then(|f| f.as_str())
        .unwrap_or("stop");
    if finish_reason != "tool_calls" {
        events.push(LlmEvent::Stop);
    }

    Ok(events)
}

// === Google Gemini Provider (API key, mirrors AnthropicProvider) ===

/// Google Gemini provider using the Generative Language REST API.
///
/// Authenticated with an API key (`GEMINI_API_KEY`, falling back to
/// `GOOGLE_API_KEY`), exactly like `AnthropicProvider` uses `ANTHROPIC_API_KEY`.
pub struct GeminiProvider {
    api_key: String,
    model: String,
    max_tokens: u32,
    base_url: String,
    client: reqwest::Client,
}

impl GeminiProvider {
    pub fn from_env() -> Result<Self, String> {
        // Accept either name — Google's own SDKs read both.
        let api_key = std::env::var("GEMINI_API_KEY")
            .or_else(|_| std::env::var("GOOGLE_API_KEY"))
            .map_err(|_| "GEMINI_API_KEY not set".to_string())?;
        if api_key.is_empty() {
            return Err("GEMINI_API_KEY is empty".to_string());
        }
        Ok(Self {
            api_key,
            model: std::env::var("OPENAB_AGENT_MODEL")
                .unwrap_or_else(|_| "gemini-2.5-flash".to_string()),
            max_tokens: std::env::var("OPENAB_AGENT_MAX_TOKENS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8192),
            base_url: std::env::var("OPENAB_AGENT_GEMINI_BASE_URL")
                .unwrap_or_else(|_| "https://generativelanguage.googleapis.com/v1beta".to_string()),
            client: reqwest::Client::new(),
        })
    }

    fn build_request_body(&self, system: &str, messages: &[Message], tools: &[ToolDef]) -> Value {
        // Map our (Anthropic-shaped) messages onto Gemini `contents`.
        let contents: Vec<Value> = messages
            .iter()
            .map(|m| {
                let role = if m.role == "assistant" {
                    "model"
                } else {
                    "user"
                };
                let parts: Vec<Value> = m
                    .content
                    .iter()
                    .map(|b| match b {
                        ContentBlock::Text { text } => json!({ "text": text }),
                        ContentBlock::ToolUse {
                            name,
                            input,
                            thought_signature,
                            ..
                        } => {
                            let mut part = json!({
                                "functionCall": { "name": name, "args": input }
                            });
                            // Gemini 3.x requires the thoughtSignature it issued on a
                            // functionCall to be echoed back on later requests in the
                            // same turn; older models simply ignore it.
                            if let Some(sig) = thought_signature {
                                part["thoughtSignature"] = json!(sig);
                            }
                            part
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } => {
                            // Gemini keys a functionResponse by the function NAME, not
                            // by an id, so recover the name from the originating
                            // tool_use block earlier in the conversation.
                            let name = gemini_tool_name(messages, tool_use_id)
                                .unwrap_or_else(|| tool_use_id.clone());
                            let response = if let Some(true) = is_error {
                                json!({ "error": content })
                            } else {
                                json!({ "result": content })
                            };
                            json!({
                                "functionResponse": { "name": name, "response": response }
                            })
                        }
                    })
                    .collect();
                json!({ "role": role, "parts": parts })
            })
            .collect();

        let mut body = json!({
            "contents": contents,
            "systemInstruction": { "parts": [{ "text": system }] },
            "generationConfig": { "maxOutputTokens": self.max_tokens },
        });

        if !tools.is_empty() {
            let decls: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": &t.name,
                        "description": &t.description,
                        "parameters": &t.input_schema
                    })
                })
                .collect();
            body["tools"] = json!([{ "functionDeclarations": decls }]);
        }

        body
    }
}

/// Find the tool name for a given synthesized tool_use id by scanning prior
/// `ToolUse` blocks (Gemini function calls carry no id of their own).
fn gemini_tool_name(messages: &[Message], id: &str) -> Option<String> {
    for m in messages {
        for b in &m.content {
            if let ContentBlock::ToolUse { id: tid, name, .. } = b {
                if tid == id {
                    return Some(name.clone());
                }
            }
        }
    }
    None
}

impl LlmProvider for GeminiProvider {
    fn chat<'a>(
        &'a self,
        system: &'a str,
        messages: &'a [Message],
        tools: &'a [ToolDef],
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<LlmEvent>>> + Send + 'a>> {
        Box::pin(async move {
            let body = self.build_request_body(system, messages, tools);
            let url = format!("{}/models/{}:generateContent", self.base_url, self.model);
            let max_retries = 3u32;

            for attempt in 0..=max_retries {
                let resp = self
                    .client
                    .post(&url)
                    .header("x-goog-api-key", &self.api_key)
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| anyhow!("HTTP request failed: {e}"))?;

                let status = resp.status();

                // Retry on 429 (rate limit) or 503 (overloaded / unavailable)
                if (status.as_u16() == 429 || status.as_u16() == 503) && attempt < max_retries {
                    let delay = std::time::Duration::from_millis(1000 * 2u64.pow(attempt));
                    tokio::time::sleep(delay).await;
                    continue;
                }

                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(anyhow!("Gemini API error {status}: {text}"));
                }

                let response: Value = resp
                    .json()
                    .await
                    .map_err(|e| anyhow!("Failed to parse response: {e}"))?;

                return parse_gemini_response(&response);
            }

            Err(anyhow!("Gemini API: max retries exceeded"))
        })
    }
}

fn parse_gemini_response(response: &Value) -> Result<Vec<LlmEvent>> {
    let mut events = Vec::new();

    // Gemini can return an error envelope even with HTTP 200.
    if let Some(err) = response.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(anyhow!("Gemini API error: {msg}"));
    }

    let candidate = response
        .get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| anyhow!("missing candidates in response"))?;

    let mut tool_call_count = 0usize;

    if let Some(parts) = candidate
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
    {
        for (idx, part) in parts.iter().enumerate() {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                if !text.is_empty() {
                    events.push(LlmEvent::Text(text.to_string()));
                }
            } else if let Some(fc) = part.get("functionCall") {
                let name = fc
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let input = fc.get("args").cloned().unwrap_or(json!({}));
                // Gemini 3.x ships a per-call thoughtSignature that must be echoed
                // back on subsequent requests; capture it so the agent loop can
                // carry it on the assistant ToolUse block.
                let thought_signature = part
                    .get("thoughtSignature")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string());
                // Synthesize a stable, unique id so the agent loop can pair the
                // eventual tool_result back to this call.
                let id = format!("call_{name}_{idx}");
                events.push(LlmEvent::ToolUse {
                    id,
                    name,
                    input,
                    thought_signature,
                });
                tool_call_count += 1;
            }
        }
    }

    // Mirror Anthropic: only emit Stop when not awaiting tool results.
    if tool_call_count == 0 {
        events.push(LlmEvent::Stop);
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_text_response() {
        let resp = json!({
            "content": [{"type": "text", "text": "Hello world"}],
            "stop_reason": "end_turn"
        });
        let events = parse_anthropic_response(&resp).unwrap();
        assert_eq!(events.len(), 2);
        match &events[0] {
            LlmEvent::Text(t) => assert_eq!(t, "Hello world"),
            _ => panic!("expected Text event"),
        }
        assert!(matches!(events[1], LlmEvent::Stop));
    }

    #[test]
    fn test_parse_tool_use_response() {
        let resp = json!({
            "content": [
                {"type": "tool_use", "id": "tu_1", "name": "read", "input": {"path": "/tmp/x"}}
            ],
            "stop_reason": "tool_use"
        });
        let events = parse_anthropic_response(&resp).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LlmEvent::ToolUse {
                id, name, input, ..
            } => {
                assert_eq!(id, "tu_1");
                assert_eq!(name, "read");
                assert_eq!(input["path"], "/tmp/x");
            }
            _ => panic!("expected ToolUse event"),
        }
    }

    #[test]
    fn test_build_request_body() {
        let provider = AnthropicProvider {
            api_key: "test".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 4096,
            client: reqwest::Client::new(),
        };
        let messages = vec![Message {
            role: "user".to_string(),
            content: vec![ContentBlock::Text {
                text: "hello".to_string(),
            }],
        }];
        let body = provider.build_request_body("system prompt", &messages, &[]);
        assert_eq!(body["model"], "claude-sonnet-4-20250514");
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["system"], "system prompt");
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn test_parse_openai_text_response() {
        let resp = json!({
            "choices": [{"message": {"content": "Hello"}, "finish_reason": "stop"}]
        });
        let events = parse_openai_response(&resp).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], LlmEvent::Text(t) if t == "Hello"));
        assert!(matches!(events[1], LlmEvent::Stop));
    }

    #[test]
    fn test_parse_openai_tool_call_response() {
        let resp = json!({
            "choices": [{"message": {
                "content": null,
                "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "read", "arguments": "{\"path\":\"x.txt\"}"}}]
            }, "finish_reason": "tool_calls"}]
        });
        let events = parse_openai_response(&resp).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LlmEvent::ToolUse {
                id, name, input, ..
            } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "read");
                assert_eq!(input["path"], "x.txt");
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn test_parse_openai_empty_choices() {
        let resp = json!({"choices": []});
        assert!(parse_openai_response(&resp).is_err());
    }

    fn gemini_provider() -> GeminiProvider {
        GeminiProvider {
            api_key: "test".to_string(),
            model: "gemini-2.5-flash".to_string(),
            max_tokens: 4096,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            client: reqwest::Client::new(),
        }
    }

    #[test]
    fn test_gemini_build_request_body_maps_roles_and_tools() {
        let messages = vec![
            Message {
                role: "user".to_string(),
                content: vec![ContentBlock::Text {
                    text: "hello".to_string(),
                }],
            },
            Message {
                role: "assistant".to_string(),
                content: vec![ContentBlock::ToolUse {
                    id: "call_bash_0".to_string(),
                    name: "bash".to_string(),
                    input: json!({ "command": "ls" }),
                    thought_signature: None,
                }],
            },
            Message {
                role: "user".to_string(),
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "call_bash_0".to_string(),
                    content: "file.txt".to_string(),
                    is_error: None,
                }],
            },
        ];
        let tools = vec![ToolDef {
            name: "bash".to_string(),
            description: "run a command".to_string(),
            input_schema: json!({ "type": "object" }),
        }];
        let body = gemini_provider().build_request_body("system prompt", &messages, &tools);

        // role mapping: user stays user, assistant -> model
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][1]["role"], "model");
        // tool_use -> functionCall
        assert_eq!(
            body["contents"][1]["parts"][0]["functionCall"]["name"],
            "bash"
        );
        // tool_result -> functionResponse keyed by recovered NAME (not the id)
        assert_eq!(
            body["contents"][2]["parts"][0]["functionResponse"]["name"],
            "bash"
        );
        assert_eq!(
            body["contents"][2]["parts"][0]["functionResponse"]["response"]["result"],
            "file.txt"
        );
        // system instruction + tool declaration shape
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            "system prompt"
        );
        assert_eq!(body["tools"][0]["functionDeclarations"][0]["name"], "bash");
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 4096);
    }

    #[test]
    fn test_gemini_tool_result_error_maps_to_error_field() {
        let messages = vec![Message {
            role: "user".to_string(),
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "unknown".to_string(),
                content: "boom".to_string(),
                is_error: Some(true),
            }],
        }];
        let body = gemini_provider().build_request_body("sys", &messages, &[]);
        // No matching tool_use -> falls back to the id as the name
        assert_eq!(
            body["contents"][0]["parts"][0]["functionResponse"]["name"],
            "unknown"
        );
        assert_eq!(
            body["contents"][0]["parts"][0]["functionResponse"]["response"]["error"],
            "boom"
        );
    }

    #[test]
    fn test_parse_gemini_text_response() {
        let resp = json!({
            "candidates": [{
                "content": { "parts": [{ "text": "Hello world" }] }
            }]
        });
        let events = parse_gemini_response(&resp).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], LlmEvent::Text(t) if t == "Hello world"));
        assert!(matches!(events[1], LlmEvent::Stop));
    }

    #[test]
    fn test_parse_gemini_function_call_response() {
        let resp = json!({
            "candidates": [{
                "content": { "parts": [
                    { "text": "let me read that" },
                    { "functionCall": { "name": "read", "args": { "path": "x.txt" } } }
                ]}
            }]
        });
        let events = parse_gemini_response(&resp).unwrap();
        // text + tool use, and NO Stop (awaiting tool result)
        assert!(matches!(&events[0], LlmEvent::Text(_)));
        match &events[1] {
            LlmEvent::ToolUse {
                id, name, input, ..
            } => {
                assert_eq!(name, "read");
                assert_eq!(id, "call_read_1");
                assert_eq!(input["path"], "x.txt");
            }
            _ => panic!("expected ToolUse event"),
        }
        assert!(!events.iter().any(|e| matches!(e, LlmEvent::Stop)));
    }

    #[test]
    fn test_parse_gemini_error_envelope() {
        let resp = json!({ "error": { "message": "API key not valid" } });
        let err = parse_gemini_response(&resp).unwrap_err();
        assert!(err.to_string().contains("API key not valid"));
    }

    #[test]
    fn test_parse_gemini_captures_thought_signature() {
        let resp = json!({
            "candidates": [{
                "content": { "parts": [
                    {
                        "functionCall": { "name": "bash", "args": { "command": "ls" } },
                        "thoughtSignature": "abc123"
                    }
                ]}
            }]
        });
        let events = parse_gemini_response(&resp).unwrap();
        match &events[0] {
            LlmEvent::ToolUse {
                name,
                thought_signature,
                ..
            } => {
                assert_eq!(name, "bash");
                assert_eq!(thought_signature.as_deref(), Some("abc123"));
            }
            _ => panic!("expected ToolUse event"),
        }
    }

    #[test]
    fn test_gemini_echoes_thought_signature_on_function_call() {
        // A Gemini 3.x assistant turn carries a thought_signature on its ToolUse;
        // it must be echoed back as `thoughtSignature` on the functionCall part.
        let messages = vec![Message {
            role: "assistant".to_string(),
            content: vec![ContentBlock::ToolUse {
                id: "call_bash_0".to_string(),
                name: "bash".to_string(),
                input: json!({ "command": "ls" }),
                thought_signature: Some("sig_xyz".to_string()),
            }],
        }];
        let body = gemini_provider().build_request_body("sys", &messages, &[]);
        let part = &body["contents"][0]["parts"][0];
        assert_eq!(part["functionCall"]["name"], "bash");
        assert_eq!(part["thoughtSignature"], "sig_xyz");
    }
}
