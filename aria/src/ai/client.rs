//! The connection to the AI endpoint, through `async-openai`.
//!
//! Every model call in the app goes through [`complete`]: one blocking entry point for worker
//! threads, backed by a small shared tokio runtime. It owns the things each caller used to repeat:
//! the endpoint settings, the reasoning level, the prompt-cache key, a global timeout, retrying
//! without an optional parameter the provider rejects, and logging token and cache usage.
//!
//! Requests are typed (`CreateChatCompletionRequest`). Responses decode into the lenient
//! [`Completion`] below via `create_byot`, because OpenAI-compatible providers return fields and
//! finish reasons that the strict OpenAI types reject.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use async_openai::config::OpenAIConfig;
use async_openai::error::OpenAIError;
use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionTools, CreateChatCompletionRequest, ReasoningEffort,
    ResponseFormat,
};
use async_openai::Client;
use serde::Deserialize;

/// Long enough for a reasoning model with a tool round, short enough that a hung connection does
/// not leave her "thinking" forever.
const TIMEOUT: Duration = Duration::from_secs(90);

/// Everything one call needs beyond the endpoint settings.
#[derive(Default)]
pub struct Request {
    pub messages: Vec<ChatCompletionRequestMessage>,
    pub tools: Vec<ChatCompletionTools>,
    /// Ask for a JSON object back.
    pub json: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// Groups requests that share a prompt prefix so the provider can reuse its cache.
    pub cache_key: Option<&'static str>,
    /// Background jobs (proactive lines, idle events) never pay for deep reasoning.
    pub light: bool,
}

// ---------------------------------------------------------------------------------- response

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Completion {
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Choice {
    pub message: Message,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Message {
    pub content: Option<String>,
    /// DeepSeek-style reasoning trace; logged, never shown.
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct ToolCall {
    pub id: String,
    pub function: FunctionCall,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub prompt_tokens_details: Option<PromptDetails>,
    /// DeepSeek reports cache hits here instead of in `prompt_tokens_details`.
    pub prompt_cache_hit_tokens: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PromptDetails {
    pub cached_tokens: Option<u32>,
}

impl Usage {
    pub fn cached(&self) -> u32 {
        self.prompt_tokens_details
            .as_ref()
            .and_then(|d| d.cached_tokens)
            .or(self.prompt_cache_hit_tokens)
            .unwrap_or(0)
    }
}

impl Completion {
    /// The first choice's text, trimmed; `None` when empty.
    pub fn text(&self) -> Option<String> {
        self.choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    }
}

// ---------------------------------------------------------------------------------- errors

#[derive(Debug, Clone, PartialEq)]
pub enum AiError {
    /// No API key is configured.
    NoKey,
    /// The key was refused (401/403).
    Auth(String),
    RateLimited(String),
    /// The request itself was refused, e.g. an image sent to a text-only model.
    Rejected(String),
    Timeout,
    Network(String),
    /// The reply could not be read.
    Decode(String),
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiError::NoKey => write!(f, "no API key is set"),
            AiError::Auth(m) => write!(f, "the API key was refused ({m})"),
            AiError::RateLimited(m) => write!(f, "rate limited ({m})"),
            AiError::Rejected(m) => write!(f, "request rejected ({m})"),
            AiError::Timeout => write!(f, "the model took too long to answer"),
            AiError::Network(m) => write!(f, "network error ({m})"),
            AiError::Decode(m) => write!(f, "unreadable reply ({m})"),
        }
    }
}

impl AiError {
    /// The model refused an image or audio part (a text-only model).
    pub fn is_media_rejection(&self) -> bool {
        let AiError::Rejected(m) = self else { return false };
        let m = m.to_lowercase();
        ["image", "vision", "multimodal", "audio", "input_audio", "content type", "image_url"]
            .iter()
            .any(|k| m.contains(k))
    }

    /// The provider rejected an optional parameter we can drop and retry without.
    fn rejected_param(&self) -> Option<OptionalParam> {
        let AiError::Rejected(m) = self else { return None };
        let m = m.to_lowercase();
        if m.contains("reasoning") {
            Some(OptionalParam::Reasoning)
        } else if m.contains("prompt_cache") || m.contains("cache_key") {
            Some(OptionalParam::CacheKey)
        } else if m.contains("response_format") || m.contains("json_object") {
            Some(OptionalParam::JsonMode)
        } else {
            None
        }
    }
}

fn classify(error: OpenAIError) -> AiError {
    match error {
        OpenAIError::ApiError(response) => {
            let api = &response.api_error;
            // The parameter name (when given) is what the retry logic looks for.
            let message = match &api.param {
                Some(param) => format!("{} (param: {param})", api.message),
                None => api.message.clone(),
            };
            match response.status_code.as_u16() {
                401 | 403 => AiError::Auth(message),
                429 => AiError::RateLimited(message),
                400..=499 => AiError::Rejected(message),
                _ => AiError::Network(format!("{} {message}", response.status_code)),
            }
        }
        OpenAIError::Reqwest(e) if e.is_timeout() => AiError::Timeout,
        OpenAIError::Reqwest(e) => match e.status().map(|s| s.as_u16()) {
            Some(401 | 403) => AiError::Auth(e.to_string()),
            Some(429) => AiError::RateLimited(e.to_string()),
            Some(400..=499) => AiError::Rejected(e.to_string()),
            _ => AiError::Network(e.to_string()),
        },
        OpenAIError::JSONDeserialize(e, body) => AiError::Decode(format!("{e}: {}", crate::util::truncate(&body, 200))),
        other => AiError::Network(other.to_string()),
    }
}

// ---------------------------------------------------------------------------------- client

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum OptionalParam {
    Reasoning,
    CacheKey,
    JsonMode,
}

/// Optional parameters this endpoint has refused once; they are not sent again this session.
static DROPPED: [AtomicBool; 3] = [AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false)];

fn dropped(param: OptionalParam) -> &'static AtomicBool {
    &DROPPED[param as usize]
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("aria-ai")
            .enable_all()
            .build()
            .expect("the AI runtime must start")
    })
}

/// The client for the current endpoint settings, rebuilt when the key or URL changes.
fn client(base_url: &str, api_key: &str) -> Client<OpenAIConfig> {
    static CACHE: Mutex<Option<(String, String, Client<OpenAIConfig>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((url, key, client)) = cache.as_ref() {
        if url == base_url && key == api_key {
            return client.clone();
        }
    }
    let config = OpenAIConfig::new().with_api_base(base_url.trim_end_matches('/')).with_api_key(api_key);
    let client = Client::with_config(config);
    *cache = Some((base_url.to_string(), api_key.to_string(), client.clone()));
    // A different endpoint may accept what the old one refused.
    for flag in &DROPPED {
        flag.store(false, Ordering::Relaxed);
    }
    client
}

/// True when an API key is configured.
pub fn available() -> bool {
    !crate::config::api_key().is_empty()
}

fn reasoning_effort(level: &str) -> Option<ReasoningEffort> {
    match level {
        "none" => Some(ReasoningEffort::None),
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        _ => None,
    }
}

fn build(request: &Request, model: String, reasoning: &str) -> CreateChatCompletionRequest {
    let effort = if request.light { reasoning_effort("low").filter(|_| reasoning != "auto") } else { reasoning_effort(reasoning) };
    CreateChatCompletionRequest {
        model,
        messages: request.messages.clone(),
        tools: (!request.tools.is_empty()).then(|| request.tools.clone()),
        response_format: (request.json && !dropped(OptionalParam::JsonMode).load(Ordering::Relaxed))
            .then_some(ResponseFormat::JsonObject),
        reasoning_effort: effort.filter(|_| !dropped(OptionalParam::Reasoning).load(Ordering::Relaxed)),
        prompt_cache_key: request
            .cache_key
            .filter(|_| !dropped(OptionalParam::CacheKey).load(Ordering::Relaxed))
            .map(str::to_string),
        max_completion_tokens: request.max_tokens,
        temperature: request.temperature,
        stream: Some(false),
        ..Default::default()
    }
}

/// Sends one chat completion and waits for it. Call from a worker thread, never the UI thread.
pub fn complete(request: &Request) -> Result<Completion, AiError> {
    let api_key = crate::config::api_key();
    if api_key.is_empty() {
        return Err(AiError::NoKey);
    }
    let settings = crate::config::get();
    let client = client(&crate::config::base_url(), &api_key);

    // At most one retry per optional parameter the endpoint turns out not to support.
    for _ in 0..4 {
        let body = build(request, settings.ai_model.clone(), &settings.reasoning);
        let started = std::time::Instant::now();
        let result = runtime().block_on(async {
            let timeout = if request.light { Duration::from_secs(8) } else { TIMEOUT };
            match tokio::time::timeout(timeout, client.chat().create_byot::<_, Completion>(body)).await {
                Ok(result) => result.map_err(classify),
                Err(_) => Err(AiError::Timeout),
            }
        });
        match result {
            Ok(completion) => {
                log_usage(&completion, started.elapsed());
                return Ok(completion);
            }
            Err(error) => match error.rejected_param() {
                Some(param) if !dropped(param).swap(true, Ordering::Relaxed) => {
                    log::warn!("the endpoint refused an optional parameter; retrying without it: {error}");
                }
                _ => return Err(error),
            },
        }
    }
    Err(AiError::Rejected("the endpoint refused every variant of the request".into()))
}

fn log_usage(completion: &Completion, took: Duration) {
    let Some(usage) = completion.usage.as_ref() else { return };
    let thought = completion
        .choices
        .first()
        .and_then(|c| c.message.reasoning_content.as_ref())
        .map(|r| format!(", {} chars of reasoning", r.len()))
        .unwrap_or_default();
    log::info!(
        "AI: {} prompt tokens ({} cached), {} completion tokens{thought} in {:.1}s",
        usage.prompt_tokens,
        usage.cached(),
        usage.completion_tokens,
        took.as_secs_f32()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_responses_decode() {
        // A DeepSeek-style reply: a reasoning trace, a finish reason the strict types do not know,
        // cache hits outside `prompt_tokens_details`, and fields nobody asked for.
        let raw = r#"{
            "id": "x", "object": "chat.completion", "created": 1, "model": "m",
            "choices": [{ "index": 0, "finish_reason": "insufficient_system_resource",
                "message": { "role": "assistant", "content": " [EMOTION: happy] Hi! ",
                    "reasoning_content": "thinking...",
                    "tool_calls": [{ "id": "c1", "type": "function",
                        "function": { "name": "get_system_stats", "arguments": "{}" } }] } }],
            "usage": { "prompt_tokens": 1200, "completion_tokens": 30, "prompt_cache_hit_tokens": 1024,
                "total_tokens": 1230, "something_new": true }
        }"#;
        let completion: Completion = serde_json::from_str(raw).expect("decodes");
        assert_eq!(completion.text().as_deref(), Some("[EMOTION: happy] Hi!"));
        assert_eq!(completion.choices[0].message.tool_calls[0].function.name, "get_system_stats");
        assert_eq!(completion.usage.as_ref().map(Usage::cached), Some(1024));
    }

    #[test]
    fn errors_are_classified_for_retries() {
        let media = AiError::Rejected("This model does not support image_url content".into());
        assert!(media.is_media_rejection());
        assert!(!AiError::Timeout.is_media_rejection());
        let param = AiError::Rejected("Unrecognized request argument supplied: reasoning_effort".into());
        assert!(param.rejected_param() == Some(OptionalParam::Reasoning));
        assert!(AiError::Auth("nope".into()).rejected_param().is_none());
    }

    /// Talks to the real endpoint with the configured key: `cargo test -p aria live_endpoint -- --ignored`.
    #[test]
    #[ignore = "calls the configured AI endpoint"]
    fn live_endpoint_answers_and_reports_cache_use() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let request = Request {
            messages: vec![
                super::super::prompt::system("Reply with exactly: pong"),
                super::super::prompt::user("ping"),
            ],
            // Room for a reasoning model to think before it answers.
            max_tokens: Some(400),
            cache_key: Some("aria-test"),
            ..Default::default()
        };
        let first = complete(&request).expect("the endpoint answers");
        if let Some(choice) = first.choices.first() {
            println!(
                "finish: {:?}, content: {:?}, reasoning chars: {}",
                choice.finish_reason,
                choice.message.content,
                choice.message.reasoning_content.as_ref().map_or(0, String::len)
            );
        }
        assert!(first.text().is_some_and(|t| t.to_lowercase().contains("pong")), "{:?}", first.text());
        let tools = super::super::tools::definitions().to_vec();
        let with_tools = Request { tools, ..request };
        let second = complete(&with_tools).expect("tool declarations are accepted");
        println!("usage: {:?}", second.usage);
    }

    #[test]
    fn requests_carry_reasoning_and_cache_key() {
        let request = Request { cache_key: Some("aria-chat"), ..Default::default() };
        let body = build(&request, "m".into(), "high");
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["reasoning_effort"], "high");
        assert_eq!(json["prompt_cache_key"], "aria-chat");
        assert_eq!(json["stream"], false);
        assert!(json.get("tools").is_none(), "no empty tools array");

        let auto = serde_json::to_value(build(&request, "m".into(), "auto")).unwrap();
        assert!(auto.get("reasoning_effort").is_none(), "auto leaves it to the model");
        let light = Request { light: true, ..Default::default() };
        let light = serde_json::to_value(build(&light, "m".into(), "high")).unwrap();
        assert_eq!(light["reasoning_effort"], "low", "background jobs think lightly");
    }
}
