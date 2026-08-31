//! The one file that touches the network.
//!
//! Everything is the OpenAI `/v1/chat/completions` shape, which is the lingua
//! franca: OpenRouter, Ollama, LM Studio and OpenAI itself all speak it, so
//! one client covers every provider and "provider" only picks defaults.
//!
//! `waki` wraps the raw `wasi:http` bindings; if it ever lags the WASI minor
//! the console links, this file is the whole surface to rewrite against the
//! raw bindings.

use serde_json::{json, Value};

pub struct ChatRequest<'a> {
    pub base_url: &'a str,
    pub model: &'a str,
    pub api_key: Option<&'a str>,
    pub system: &'a str,
    pub user: &'a str,
}

/// One round trip: the assistant's reply text, or an error worth showing.
pub fn chat(request: &ChatRequest) -> Result<String, String> {
    let url = format!("{}/chat/completions", request.base_url.trim_end_matches('/'));
    let body = json!({
        "model": request.model,
        "messages": [
            { "role": "system", "content": request.system },
            { "role": "user", "content": request.user },
        ],
        "temperature": 0.2,
    });

    let mut builder = waki::Client::new()
        .post(&url)
        .header("content-type", "application/json")
        .connect_timeout(std::time::Duration::from_secs(60));
    if let Some(key) = request.api_key {
        builder = builder.header("authorization", &format!("Bearer {key}"));
    }
    let response = builder
        .json(&body)
        .send()
        .map_err(|e| format!("could not reach {url}: {e}"))?;

    let status = response.status_code();
    let bytes = response.body().map_err(|e| format!("reading the reply: {e}"))?;
    let parsed: Value = serde_json::from_slice(&bytes)
        .map_err(|_| format!("{url} answered {status} with something that is not JSON"))?;

    if !(200..300).contains(&status) {
        let detail = parsed
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("no detail given");
        return Err(format!("{url} answered {status}: {detail}"));
    }

    parsed
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "the model's reply had no content".to_string())
}
