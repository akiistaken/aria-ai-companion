//! The conversation: history, the tool loop, and turning the model's reply into what she says,
//! feels and remembers.

use std::sync::Mutex;

use async_openai::types::chat::ChatCompletionRequestMessage;

use super::client::{self, AiError, Request};
use super::prompt;
use super::tools::{self, ToolCtx};
use super::Reply;

/// Messages kept between turns (user, assistant and tool messages all count).
const HISTORY_LIMIT: usize = 24;
/// Tool rounds per turn before she has to answer.
const MAX_ROUNDS: usize = 6;

static HISTORY: Mutex<Vec<ChatCompletionRequestMessage>> = Mutex::new(Vec::new());

/// Forgets the conversation (not the long-term memory vault).
pub fn reset() {
    HISTORY.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

fn history() -> Vec<ChatCompletionRequestMessage> {
    HISTORY.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Appends one finished turn and trims the oldest whole turns: a turn always starts with a user
/// message, so trimming never leaves a tool result without the call that asked for it.
fn remember_turn(turn: Vec<ChatCompletionRequestMessage>) {
    let mut h = HISTORY.lock().unwrap_or_else(|e| e.into_inner());
    h.extend(turn);
    while h.len() > HISTORY_LIMIT {
        let next_turn = h.iter().skip(1).position(|m| matches!(m, ChatCompletionRequestMessage::User(_)));
        match next_turn {
            Some(i) => {
                h.drain(..=i);
            }
            None => {
                h.clear();
            }
        }
    }
}

/// Words that mean "look at my screen".
fn wants_screen(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    ["screen", "looking at", "what am i", "what's on", "whats on", "see my", "look at", "this window", "watching"]
        .iter()
        .any(|k| lower.contains(k))
}

/// "switch to yandere", "be genki", …: the persona the user asked for, if any.
fn requested_persona(message: &str) -> Option<&'static str> {
    let lower = message.trim().to_lowercase();
    let extra: &[(&str, &[&str])] = &[
        ("bakadere", &["be goofy", "be clumsy"]),
        ("kuudere", &["be cold", "be quiet"]),
        ("genki", &["genki girl", "hyper mode"]),
        ("meganekko", &["glasses mode", "be smart"]),
    ];
    crate::persona::KEYS.iter().copied().find(|key| {
        lower == *key
            || [format!("switch to {key}"), format!("{key} mode"), format!("be {key}"), format!("be a {key}"), format!("persona {key}")]
                .iter()
                .any(|p| lower.contains(p.as_str()))
            || extra.iter().any(|(k, phrases)| k == key && phrases.iter().any(|p| lower.contains(p)))
    })
}

/// One conversational turn. Runs on a worker thread; `move_mascot` posts to the UI thread.
pub fn chat(user_message: &str, include_screen: bool, move_mascot: &dyn Fn(&str, Option<&str>)) -> Reply {
    let mut logs = Vec::new();
    crate::persona::mood::note_user_text(user_message);
    // Asking for a persona switches it before the model answers, so the answer is already in it.
    if let Some(key) = requested_persona(user_message) {
        crate::persona::mood::set_direct_mood(key, "you asked for this side of me");
        logs.push(format!("Mood: switched to {} on request", crate::util::capitalize(key)));
    }
    if !client::available() {
        return Reply::unavailable(&AiError::NoKey).logs(logs);
    }

    let persona = crate::persona::current();
    let mut system = prompt::system_prompt(persona);
    let notes = tools::desktop_notes();
    if !notes.is_empty() {
        system.push_str("\n\n");
        system.push_str(notes);
    }

    let screenshot = (include_screen || wants_screen(user_message))
        .then(|| crate::desktop::win32::capture_screen_jpeg_b64(1280))
        .flatten();
    if screenshot.is_some() {
        logs.push("Captured live desktop screenshot".into());
    }
    let user_turn = match &screenshot {
        Some(b64) => prompt::user_with_image(user_message, "image/jpeg", b64),
        None => prompt::user(user_message),
    };

    // Stable prefix first, then history, then this turn's context and message (see `prompt`).
    let mut messages = vec![prompt::system(system)];
    messages.extend(history());
    let prefix_len = messages.len();
    messages.push(prompt::system(prompt::context_block()));
    messages.push(user_turn);

    let ctx = ToolCtx { move_mascot };
    let mut final_text = String::new();
    let mut retried_without_image = false;
    let mut retried_empty = false;
    let mut round = 0;
    while round < MAX_ROUNDS {
        let request = Request {
            messages: messages.clone(),
            tools: tools::definitions().to_vec(),
            cache_key: Some("aria-chat"),
            ..Default::default()
        };
        let completion = match client::complete(&request) {
            Ok(c) => c,
            // A text-only model refused the screenshot: say so and ask again without it.
            Err(e) if e.is_media_rejection() && screenshot.is_some() && !retried_without_image => {
                retried_without_image = true;
                log::warn!("the model rejected the screenshot, retrying text-only: {e}");
                // The user turn sits right after this turn's context block.
                messages[prefix_len + 1] = prompt::user(format!("{user_message} (A screenshot was captured, but this model cannot see images.)"));
                continue;
            }
            Err(e) => {
                log::error!("AI chat error: {e}");
                return Reply::unavailable(&e).logs(logs);
            }
        };
        let Some(choice) = completion.choices.into_iter().next() else {
            return Reply::unavailable(&AiError::Decode("no choices".into())).logs(logs);
        };
        let message = choice.message;
        let text = message.content.clone().filter(|t| !t.trim().is_empty());
        messages.push(prompt::assistant(text.clone(), &message.tool_calls));
        let answered = text.is_some();
        if let Some(text) = text {
            final_text = text;
        }
        if message.tool_calls.is_empty() {
            // A reasoning model occasionally spends its turn thinking and says nothing; ask once more.
            if !answered && final_text.is_empty() && !retried_empty {
                retried_empty = true;
                log::warn!("empty reply (finish: {:?}); asking again", choice.finish_reason);
                messages.pop();
                continue;
            }
            break;
        }
        for call in &message.tool_calls {
            let args: serde_json::Value = serde_json::from_str(&call.function.arguments).unwrap_or_else(|_| serde_json::json!({}));
            logs.push(format!("{}({})", call.function.name, if args == serde_json::json!({}) { String::new() } else { args.to_string() }));
            let result = tools::execute(&call.function.name, &args, &ctx);
            messages.push(prompt::tool_result(&call.id, result.to_string()));
        }
        round += 1;
    }

    // Keep the turn without its screenshot (images are large and stale by the next turn) and
    // without the per-turn context block, so the next request's prefix matches this one's.
    let mut turn = vec![prompt::user(user_message)];
    turn.extend(messages.drain(prefix_len + 2..));
    remember_turn(turn);

    let tagged = prompt::read_tags(&final_text, "talking");
    if let Some(mood) = &tagged.mood {
        crate::persona::mood::set_direct_mood(mood, "Aria decided her mood in conversation");
        logs.push(format!("Mood shift: Aria adopted {} mood", crate::util::capitalize(mood)));
    }
    award_emotion_exp(&tagged.emotion);
    if !logs.iter().any(|l| l.starts_with("remember_user_fact")) {
        super::facts::remember_from(user_message);
    }
    if tagged.text.is_empty() {
        return Reply::new(crate::persona::flav("I didn't have anything to say to that!"), "annoyed").logs(logs);
    }
    Reply::new(tagged.text, tagged.emotion).logs(logs)
}

fn award_emotion_exp(emotion: &str) {
    let (key, amount) = match emotion {
        "happy" | "idle" | "laugh" | "shy" | "wave" | "clap" => ("dere", 3),
        "annoyed" | "angry" => ("tsun", 3),
        "dance" | "twirl" | "surprised" => ("hype", 3),
        "groove" | "hum" => ("groove", 4),
        "sit" | "sleepy" | "sad" | "cry" | "thinking" => ("devotion", 3),
        _ => ("dere", 1),
    };
    crate::memory::adjust_emotion(key, amount);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_keywords_trigger_capture() {
        assert!(wants_screen("what do you see on my screen?"));
        assert!(!wants_screen("tell me a joke"));
    }

    #[test]
    fn persona_requests_are_recognised() {
        assert_eq!(requested_persona("switch to yandere please"), Some("yandere"));
        assert_eq!(requested_persona("Genki"), Some("genki"));
        assert_eq!(requested_persona("be goofy"), Some("bakadere"));
        assert_eq!(requested_persona("what's the weather"), None);
    }

    /// A whole turn against the configured endpoint, with a real tool call:
    /// `cargo test -p aria live_chat -- --ignored --nocapture`.
    #[test]
    #[ignore = "calls the configured AI endpoint"]
    fn live_chat_uses_tools_and_tags() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        reset();
        let reply = chat("What song is playing on my PC right now? Use your tool to check.", false, &|_, _| {});
        println!("reply: [{}] {}\nlogs: {:?}", reply.emotion, reply.text, reply.logs);
        assert!(reply.logs.iter().any(|l| l.starts_with("get_now_playing_media")), "the model checked with its tool");
        assert!(aria_protocol::EMOTIONS.contains(&reply.emotion.as_str()) || reply.emotion == "walking");
        assert!(!reply.text.contains("[EMOTION"), "tags are stripped");
        assert!(!history().is_empty(), "the turn is remembered");
        reset();
    }

    #[test]
    fn history_trims_whole_turns() {
        reset();
        for i in 0..10 {
            remember_turn(vec![
                prompt::user(format!("q{i}")),
                prompt::assistant(None, &[client::ToolCall {
                    id: format!("c{i}"),
                    function: client::FunctionCall { name: "get_system_stats".into(), arguments: "{}".into() },
                }]),
                prompt::tool_result(&format!("c{i}"), "{}".into()),
                prompt::assistant(Some(format!("a{i}")), &[]),
            ]);
        }
        let h = history();
        assert!(h.len() <= HISTORY_LIMIT);
        assert!(matches!(h[0], ChatCompletionRequestMessage::User(_)), "history starts on a user turn");
        reset();
    }
}
