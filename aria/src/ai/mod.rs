//! Everything that talks to the AI endpoint.
//!
//! | Module | What |
//! |---|---|
//! | [`client`] | the `async-openai` client: endpoint, reasoning level, prompt-cache key, timeouts, retries |
//! | [`prompt`] | the system prompt, the per-turn context block, message builders, reading emotion/mood tags |
//! | [`brain`] | the conversation and its tool loop |
//! | [`tools`] | the tool declarations and what they do (including Cua Driver's) |
//! | [`tasks`] | one-shot jobs: clipboard diagnosis, audio, visual grounding, proactive lines, idle events |
//! | [`facts`] | picking personal facts out of what the user says |
//!
//! There is no offline mode: every reply comes from the endpoint. Without an API key she says
//! so and points at the Control Center. Instant touch reactions (poke, head-pat) stay scripted
//! because they answer a gesture in the same frame.

pub mod brain;
pub mod client;
pub mod facts;
pub mod prompt;
pub mod tasks;
pub mod tools;

pub use brain::{chat, reset};
pub use client::AiError;

/// What she says back, how she feels about it, and what happened on the way.
#[derive(Debug, Clone)]
pub struct Reply {
    pub text: String,
    pub emotion: String,
    pub logs: Vec<String>,
}

impl Reply {
    pub fn new(text: impl Into<String>, emotion: impl Into<String>) -> Self {
        Self { text: text.into(), emotion: emotion.into(), logs: Vec::new() }
    }

    pub fn logs(mut self, logs: Vec<String>) -> Self {
        self.logs = logs;
        self
    }

    /// A model reply with its tags read.
    pub fn from_model(raw: &str, default_emotion: &str) -> Self {
        let tagged = prompt::read_tags(raw, default_emotion);
        Self::new(tagged.text, tagged.emotion)
    }

    /// What she says when the endpoint cannot answer, in character.
    pub fn unavailable(error: &AiError) -> Self {
        let line = match error {
            AiError::NoKey => "I need an API key before I can think! Add one in the Control Center under Engine.".to_string(),
            AiError::Auth(_) => "My API key was refused. Check it in the Control Center under Engine.".to_string(),
            AiError::RateLimited(_) => "The AI endpoint says I'm talking too much. Give me a moment and try again.".to_string(),
            AiError::Timeout => "My thoughts timed out. Try again in a moment.".to_string(),
            AiError::Network(_) => "I can't reach the AI endpoint. Is the internet connection up?".to_string(),
            AiError::Rejected(m) | AiError::Decode(m) => {
                format!("Something went wrong with my neural link ({}).", crate::util::truncate(m, 100))
            }
        };
        Self::new(crate::persona::flav(&line), "annoyed").logs(vec![format!("AI unavailable: {error}")])
    }
}
