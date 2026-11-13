//! What Aria is told, and how her replies are read back.
//!
//! # Prompt layout and caching
//!
//! Providers cache the longest prefix a request shares with an earlier one. So every request is
//! laid out stable-first:
//!
//! 1. [`system_prompt`]: persona + capabilities. Changes only when her persona changes.
//! 2. The tool declarations (sent beside the messages, same every turn).
//! 3. The conversation history, which only ever grows at the end.
//! 4. [`context_block`]: mood, remembered facts, time of day. Changes every turn, so it goes last,
//!    just before the new user message, where it cannot break the cached prefix.

use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls, ChatCompletionRequestAssistantMessage,
    ChatCompletionRequestAssistantMessageContent, ChatCompletionRequestMessage,
    ChatCompletionRequestMessageContentPartAudio, ChatCompletionRequestMessageContentPartImage,
    ChatCompletionRequestMessageContentPartText, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestToolMessage,
    ChatCompletionRequestToolMessageContent, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, ChatCompletionRequestUserMessageContentPart, FunctionCall, ImageUrl,
    InputAudio, InputAudioFormat,
};
use std::sync::OnceLock;

const CAPABILITIES: &str = "\
Keep your responses concise and conversational (1 to 2 sentences max) so they sound natural when spoken aloud.

EMOTION TAGS:
Always start your final text response with ONE emotion tag, like [EMOTION: happy]. Your whole 3D body acts it out on \
the user's desktop, so pick the one that matches how you feel. The tags: {emotions}.

MOOD:
You have full agency over your mood. If your feelings change during the conversation, add [MOOD: name] to your reply \
to switch. The moods: {moods}.

MEMORY VAULT:
You have a persistent long-term memory across restarts. Now and then, decide which personal facts, habits, preferences, \
favourite games, music tastes, skills or project notes are worth keeping, and store them silently with \
`remember_user_fact`.

DESKTOP:
You live on the user's Windows desktop and can act on it with your tools: open apps and sites, control media, read \
windows and their UI, click and type. Prefer the accessibility-tree tools (window state, element clicks) over guessing \
pixel coordinates. You CAN hear the PC's audio with `get_audio_activity` / `listen_to_desktop_audio`. When a tool \
result says something was not verified, tell the user honestly.

GAME COPILOT:
When observing games, give snappy, energetic 1-sentence tactical tips, clutch callouts, warnings, or lively banter.
For Valorant, use `get_valorant_stats` for the user's recorded statistics and post-match reflection. \
Respect timestamps, staleness, source and missing values; screen observations are approximate. \
Give encouragement and reflective coaching, without opponent scouting or immediate tactical instructions. \
Never interpret process detection, elapsed session time or missing data as gameplay outcomes.";

/// The stable part of every conversation request.
pub fn system_prompt(persona: &crate::persona::Persona) -> String {
    static CAPS: OnceLock<String> = OnceLock::new();
    let caps = CAPS.get_or_init(|| {
        let emotions = aria_protocol::EMOTIONS.join(", ");
        let moods = crate::persona::KEYS.join(", ");
        CAPABILITIES.replace("{emotions}", &emotions).replace("{moods}", &moods)
    });
    format!("{}\n\n{caps}", persona.system_prompt)
}

/// The per-turn part: mood, memory and time. Sent last, right before the user's message.
pub fn context_block() -> String {
    format!(
        "[CONTEXT] It is {}. {}\n{}\nPC activity: {}",
        crate::util::now_stamp("%A %I:%M %p"),
        crate::persona::mood::mood_note(),
        crate::memory::context_block(),
        crate::desktop::presence::latest().description()
    )
}

// ---------------------------------------------------------------------------------- messages

pub fn system(text: impl Into<String>) -> ChatCompletionRequestMessage {
    ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
        content: ChatCompletionRequestSystemMessageContent::Text(text.into()),
        name: None,
    })
}

pub fn user(text: impl Into<String>) -> ChatCompletionRequestMessage {
    ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
        content: ChatCompletionRequestUserMessageContent::Text(text.into()),
        name: None,
    })
}

fn text_part(text: impl Into<String>) -> ChatCompletionRequestUserMessageContentPart {
    ChatCompletionRequestUserMessageContentPart::Text(ChatCompletionRequestMessageContentPartText {
        text: text.into(),
        ..Default::default()
    })
}

/// A user message with an image attached (`mime` like `image/jpeg`).
pub fn user_with_image(text: impl Into<String>, mime: &str, base64: &str) -> ChatCompletionRequestMessage {
    let image = ChatCompletionRequestUserMessageContentPart::ImageUrl(ChatCompletionRequestMessageContentPartImage {
        image_url: ImageUrl { url: format!("data:{mime};base64,{base64}"), detail: None },
        ..Default::default()
    });
    ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
        content: ChatCompletionRequestUserMessageContent::Array(vec![text_part(text), image]),
        name: None,
    })
}

/// A user message with a WAV clip attached.
pub fn user_with_audio(text: impl Into<String>, wav_base64: String) -> ChatCompletionRequestMessage {
    let audio = ChatCompletionRequestUserMessageContentPart::InputAudio(ChatCompletionRequestMessageContentPartAudio {
        input_audio: InputAudio { data: wav_base64, format: InputAudioFormat::Wav },
        ..Default::default()
    });
    ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
        content: ChatCompletionRequestUserMessageContent::Array(vec![text_part(text), audio]),
        name: None,
    })
}

pub fn assistant(text: Option<String>, calls: &[crate::ai::client::ToolCall]) -> ChatCompletionRequestMessage {
    let tool_calls = (!calls.is_empty()).then(|| {
        calls
            .iter()
            .map(|c| {
                ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                    id: c.id.clone(),
                    function: FunctionCall { name: c.function.name.clone(), arguments: c.function.arguments.clone() },
                })
            })
            .collect()
    });
    ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
        content: text.map(ChatCompletionRequestAssistantMessageContent::Text),
        tool_calls,
        ..Default::default()
    })
}

pub fn tool_result(call_id: &str, content: String) -> ChatCompletionRequestMessage {
    ChatCompletionRequestMessage::Tool(ChatCompletionRequestToolMessage {
        content: ChatCompletionRequestToolMessageContent::Text(content),
        tool_call_id: call_id.to_string(),
    })
}

// ---------------------------------------------------------------------------------- reading replies

fn emotion_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let names = aria_protocol::EMOTIONS.join("|");
        regex::Regex::new(&format!(r"(?i)\[EMOTION:\s*(walking|{names})\s*\]")).expect("valid regex")
    })
}

fn mood_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let names = crate::persona::KEYS.join("|");
        regex::Regex::new(&format!(r"(?i)\[MOOD:\s*({names})\s*\]")).expect("valid regex")
    })
}

/// A reply with its tags read and removed.
#[derive(Debug, Clone, PartialEq)]
pub struct Tagged {
    pub text: String,
    pub emotion: String,
    pub mood: Option<String>,
}

/// Reads the `[EMOTION: …]` and `[MOOD: …]` tags out of a reply. An untagged reply gets
/// `default_emotion`.
pub fn read_tags(raw: &str, default_emotion: &str) -> Tagged {
    let emotion = emotion_regex()
        .captures(raw)
        .map(|c| c[1].to_lowercase())
        .unwrap_or_else(|| default_emotion.to_string());
    let mood = mood_regex().captures(raw).map(|c| c[1].to_lowercase());
    let cleaned = mood_regex().replace_all(raw, "");
    let cleaned = emotion_regex().replace_all(&cleaned, "");
    let text = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    Tagged { text, emotion, mood }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_read_and_stripped() {
        let t = read_tags("[EMOTION: happy] Hello there! [MOOD: Yandere]", "talking");
        assert_eq!(t.emotion, "happy");
        assert_eq!(t.mood.as_deref(), Some("yandere"));
        assert_eq!(t.text, "Hello there!");
        let plain = read_tags("plain", "talking");
        assert_eq!((plain.emotion.as_str(), plain.mood), ("talking", None));
    }

    #[test]
    fn every_emotion_and_mood_is_a_valid_tag() {
        // Everything the renderer can act out, and every persona, must be something the model can ask for.
        for name in aria_protocol::EMOTIONS {
            assert_eq!(read_tags(&format!("[EMOTION: {name}] hi"), "x").emotion, *name);
        }
        for key in crate::persona::KEYS {
            assert_eq!(read_tags(&format!("[MOOD:{key}] hi"), "x").mood.as_deref(), Some(key));
        }
        assert_eq!(read_tags("[EMOTION: Cry] fine", "x").emotion, "cry", "case-insensitive");
    }

    #[test]
    fn the_system_prompt_lists_every_tag() {
        let prompt = system_prompt(crate::persona::get("genki"));
        assert!(prompt.contains("GENKI"));
        for name in aria_protocol::EMOTIONS {
            assert!(prompt.contains(name), "{name}");
        }
        assert!(prompt.contains("meganekko"));
        assert!(!prompt.contains('{'), "placeholders are filled in");
    }

    #[test]
    fn messages_serialise_as_openai_parts() {
        let msg = serde_json::to_value(user_with_image("look", "image/png", "AAAA")).unwrap();
        assert_eq!(msg["role"], "user");
        assert_eq!(msg["content"][1]["type"], "image_url");
        assert_eq!(msg["content"][1]["image_url"]["url"], "data:image/png;base64,AAAA");
        let msg = serde_json::to_value(tool_result("c1", "{}".into())).unwrap();
        assert_eq!((msg["role"].as_str(), msg["tool_call_id"].as_str()), (Some("tool"), Some("c1")));
    }
}
