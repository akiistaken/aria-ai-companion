//! One-shot model jobs outside the conversation: clipboard diagnosis, describing desktop audio,
//! finding things on screen, writing proactive lines, and choosing what she does when idle.
//!
//! Each reuses the conversation's stable system prompt where the reply is in character, so these
//! calls share the cached prefix too.

use serde::Deserialize;
use serde_json::{json, Value};

use super::client::{self, AiError, Request};
use super::prompt;
use super::Reply;

/// One request, one text answer.
fn ask(messages: Vec<async_openai::types::chat::ChatCompletionRequestMessage>, json: bool, light: bool) -> Result<String, AiError> {
    let request = Request {
        messages,
        json,
        light,
        cache_key: Some("aria-tasks"),
        // Generous: on reasoning models the thinking counts against this too.
        max_tokens: Some(1500),
        ..Default::default()
    };
    client::complete(&request)?.text().ok_or_else(|| AiError::Decode("empty reply".into()))
}

fn in_character(instruction: String) -> Vec<async_openai::types::chat::ChatCompletionRequestMessage> {
    vec![
        prompt::system(prompt::system_prompt(crate::persona::current())),
        prompt::system(prompt::context_block()),
        prompt::user(instruction),
    ]
}

/// Explains an error the user just copied to the clipboard.
pub fn analyze_clipboard(clip: &str) -> Reply {
    let last = clip.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or(clip).to_string();
    let log = format!("Clipboard bug diagnosis: {}", crate::util::truncate(&last, 48));
    let instruction = format!(
        "The user just copied this error/traceback to their clipboard. Give a short 2-sentence explanation of \
         the bug and how to fix it, in character:\n\n{}",
        crate::util::truncate(clip, 2500)
    );
    match ask(in_character(instruction), false, false) {
        Ok(text) => Reply::from_model(&text, "thinking").logs(vec![log]),
        Err(e) => Reply::unavailable(&e).logs(vec![log]),
    }
}

/// "What do you hear?": loopback capture, described by the model.
pub fn listen_to_desktop_audio(seconds: f64, question: &str) -> Value {
    let activity = crate::desktop::audio::audio_activity(240);
    let (samples, levels) = crate::desktop::audio::capture_desktop_audio(seconds);
    let audible_apps: Vec<String> = activity.apps.iter().map(|a| a.name.clone()).collect();
    let mut result = json!({ "status": "success", "audible_apps": audible_apps, "levels": levels });

    let heard = if samples.is_empty() {
        result["status"] = json!("error");
        levels.error.clone().unwrap_or_else(|| "Could not capture desktop audio".into())
    } else if levels.silent {
        "Silence — nothing is playing through the speakers right now.".into()
    } else {
        let wav = crate::desktop::audio::wav_bytes(&samples, 16_000);
        let prompt_text = if question.trim().is_empty() {
            "This is a clip of the user's desktop audio. Briefly say what it is: if music, the likely song/artist \
             (only if you recognise it), genre and mood; if speech or video, what is being said; if a game, what it \
             sounds like. Two short sentences."
                .to_string()
        } else {
            question.trim().to_string()
        };
        match ask(vec![prompt::user_with_audio(prompt_text, crate::util::base64(&wav))], false, true) {
            Ok(text) => text,
            Err(e) if e.is_media_rejection() => {
                let playing = crate::desktop::media::now_playing();
                format!(
                    "Audio is playing ({} dBFS{}), but this model cannot listen to audio clips.",
                    levels.dbfs,
                    if playing.is_playing { format!(", the player says: {}", playing.label()) } else { String::new() }
                )
            }
            Err(e) => format!("Audio is playing ({} dBFS) but the analysis failed: {e}", levels.dbfs),
        }
    };
    result["heard"] = json!(heard);
    result
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Located {
    found: bool,
    label: String,
    box_2d: Vec<f64>,
}

/// Finds a UI element on a screenshot: `(x, y, label)` in screen pixels.
pub fn locate_on_screen(description: &str) -> Option<(i32, i32, String)> {
    let b64 = crate::desktop::win32::capture_screen_jpeg_b64(1366)?;
    let (mx, my, mw, mh) = crate::desktop::win32::screen_size();
    let instruction = format!(
        "Locate the UI element on this desktop screenshot matching: '{description}'. Return JSON: \
         {{\"found\": true, \"label\": \"name of element\", \"box_2d\": [ymin, xmin, ymax, xmax]}} with integers \
         normalized to 0-1000. Ignore any desktop companion mascot overlay. If it is not visible, return \
         {{\"found\": false, \"label\": \"\", \"box_2d\": [0,0,0,0]}}."
    );
    let messages = vec![
        prompt::system("You are a precise desktop visual grounding assistant. Return only valid JSON."),
        prompt::user_with_image(instruction, "image/jpeg", &b64),
    ];
    let text = ask(messages, true, true).map_err(|e| log::warn!("visual grounding failed: {e}")).ok()?;
    let located: Located = serde_json::from_str(json_body(&text)).ok()?;
    if !located.found || located.box_2d.len() != 4 {
        return None;
    }
    let b = &located.box_2d;
    let cx = (b[1] + b[3]) / 2.0 / 1000.0;
    let cy = (b[0] + b[2]) / 2.0 / 1000.0;
    let label = if located.label.is_empty() { description.to_string() } else { located.label };
    Some(((mx as f64 + cx * mw as f64) as i32, (my as f64 + cy * mh as f64) as i32, label))
}

/// Finds and clicks something described in words. Returns how (for tool results).
pub fn click_described(description: &str) -> String {
    match locate_on_screen(description) {
        Some((x, y, _)) => {
            crate::desktop::input::move_cursor(x, y);
            std::thread::sleep(std::time::Duration::from_millis(300));
            crate::desktop::input::click(x, y, false, false);
            "vision_click".into()
        }
        None => "vision_click_failed".into(),
    }
}

/// Models sometimes wrap JSON in a code fence or a sentence; take the outermost object.
fn json_body(text: &str) -> &str {
    match (text.find('{'), text.rfind('}')) {
        (Some(start), Some(end)) if end > start => &text[start..=end],
        _ => text,
    }
}

/// Writes her line for a proactive moment (a game started, the user came back, …) in character.
/// `None` when unavailable or repetitive; background moments stay silent.
pub fn proactive_line(situation: &str, emotion_hint: &str) -> Option<(String, String)> {
    for _ in 0..2 {
        let instruction = format!(
            "Something just happened on the user's desktop. React to it out loud, unprompted, in ONE short spoken \
             sentence (under 25 words), in character. Start with an emotion tag; a fitting one here would be \
             [EMOTION: {emotion_hint}]. Use fresh wording and a different observation each time.\n\n\
             What happened: {situation}\nRecent lines (do not repeat or paraphrase): {}",
             recent_lines().join(" | ")
        );
        match ask(in_character(instruction), false, true) {
            Ok(text) => {
                let tagged = prompt::read_tags(&text, emotion_hint);
                if remember_line(&tagged.text) {
                    return Some((tagged.text, tagged.emotion));
                }
            }
            Err(e) => {
                log::debug!("background speech unavailable: {e}");
                return None;
            }
        }
    }
    None
}

/// Persist a bounded history so even greetings vary between app launches. Never hold this lock
/// during a network request; accepting a line is atomic across background jobs.
fn line_history() -> &'static std::sync::Mutex<Vec<String>> {
    static HISTORY: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    HISTORY.get_or_init(|| {
        let saved = std::fs::read(crate::config::data_dir().join("reaction-history.json"))
            .ok().and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok()).unwrap_or_default();
        std::sync::Mutex::new(saved.into_iter().rev().take(64).collect::<Vec<_>>().into_iter().rev().collect())
    })
}

fn recent_lines() -> Vec<String> {
    line_history().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn line_key(line: &str) -> String {
    line.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn remember_line(line: &str) -> bool {
    let key = line_key(line);
    if key.is_empty() { return false; }
    let mut history = line_history().lock().unwrap_or_else(|e| e.into_inner());
    if history.iter().any(|old| line_key(old) == key) { return false; }
    history.push(line.to_string());
    if history.len() > 64 { history.remove(0); }
    if let Ok(bytes) = serde_json::to_vec(&*history) {
        if let Err(e) = std::fs::write(crate::config::data_dir().join("reaction-history.json"), bytes) {
            log::debug!("reaction history: {e}");
        }
    }
    true
}

// ---------------------------------------------------------------------------------- idle events

/// Things she can do on her own, as `(emotion, what it looks like)`. Each is a full-body animation
/// in the renderer.
pub const IDLE_ACTIONS: &[(&str, &str)] = &[
    ("stretch", "stretches her arms up over her head"),
    ("look_around", "looks around the desktop curiously"),
    ("hum", "hums to herself, swaying"),
    ("twirl", "does a little twirl on the spot"),
    ("fix_hair", "fixes her hair"),
    ("clap", "claps her hands, pleased"),
    ("wave", "waves at the user"),
    ("sit", "sits down on the taskbar for a while"),
    ("sleepy", "gets drowsy and nods off"),
    ("thinking", "ponders something"),
    ("dance", "breaks into a short dance"),
    ("shy", "gets bashful for no reason"),
    ("laugh", "remembers something funny and giggles"),
];

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct IdleEvent {
    pub action: String,
    /// What she says, if anything. Most idle moments are silent.
    pub line: String,
    pub walk: String,
}

impl IdleEvent {
    /// Keeps only what the app can act out.
    fn validated(mut self) -> Option<IdleEvent> {
        self.action = self.action.trim().to_lowercase();
        if !IDLE_ACTIONS.iter().any(|(a, _)| *a == self.action) {
            return None;
        }
        // Idle gestures never start a walk, even if an endpoint returns an old schema.
        self.walk = "none".into();
        self.line = crate::util::truncate(prompt::read_tags(&self.line, "idle").text.trim(), 140);
        Some(self)
    }
}

/// Asks the model what she does next while nothing is going on, given her mood and what the
/// user is up to.
pub fn idle_event(activity: &str, recent: &[String]) -> Option<IdleEvent> {
    let actions = IDLE_ACTIONS.iter().map(|(a, d)| format!("{a} ({d})")).collect::<Vec<_>>().join(", ");
    let recent = if recent.is_empty() { "nothing yet".to_string() } else { recent.join(", ") };
    let instruction = format!(
        "Nothing is going on right now; you are idling on the user's desktop. Decide what you do next, the way \
         someone with your current mood and personality would. The user is: {activity}. You recently did: \
         {recent} (avoid repeating them).\n\nChoose ONE action from: {actions}. Stay in your current spot; never walk.\n\
         Usually stay quiet; say a short line (under 15 words) only about one time in three, and never \
         interrupt the user's focus with a question.\n\nReply with JSON only: \
         {{\"action\": \"...\", \"walk\": \"none\", \"line\": \"\"}}\n\
         Recent lines (do not repeat or paraphrase): {}", recent_lines().join(" | ")
    );
    let text = ask(in_character(instruction), true, true).map_err(|e| log::debug!("idle event: {e}")).ok()?;
    let mut event = serde_json::from_str::<IdleEvent>(json_body(&text)).ok()?.validated()?;
    if !event.line.is_empty() && !remember_line(&event.line) { event.line.clear(); }
    Some(event)
}

/// A local stand-in when the endpoint is unavailable: an action that suits the persona, silent.
pub fn idle_fallback(persona: &str) -> IdleEvent {
    let pool: &[&str] = match persona {
        "genki" => &["twirl", "dance", "clap", "wave", "stretch"],
        "kuudere" => &["look_around", "thinking", "sit", "fix_hair"],
        "yandere" => &["wave", "shy", "fix_hair", "hum"],
        "tomboy" => &["stretch", "look_around", "clap", "dance"],
        "bakadere" => &["twirl", "laugh", "look_around", "hum"],
        "meganekko" => &["thinking", "fix_hair", "look_around", "sit"],
        _ => &["fix_hair", "look_around", "stretch", "hum"],
    };
    IdleEvent { action: crate::util::pick_str(pool).to_string(), line: String::new(), walk: "none".into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_events_are_validated() {
        let ok = IdleEvent { action: " Stretch".into(), line: "[EMOTION: happy] Mmmh~".into(), walk: "LEFT".into() };
        let ok = ok.validated().expect("valid");
        assert_eq!((ok.action.as_str(), ok.walk.as_str(), ok.line.as_str()), ("stretch", "none", "Mmmh~"));
        let odd_walk = IdleEvent { action: "hum".into(), walk: "to the moon".into(), ..Default::default() };
        assert_eq!(odd_walk.validated().unwrap().walk, "none");
        assert!(IdleEvent { action: "explode".into(), ..Default::default() }.validated().is_none());
    }

    #[test]
    fn every_idle_action_is_something_the_renderer_acts_out() {
        for (action, _) in IDLE_ACTIONS {
            assert!(aria_protocol::EMOTIONS.contains(action), "{action} is not an emotion the renderer knows");
        }
        for persona in crate::persona::KEYS {
            let event = idle_fallback(persona);
            assert!(IDLE_ACTIONS.iter().any(|(a, _)| *a == event.action), "{persona}: {}", event.action);
        }
    }

    /// `cargo test -p aria live_background -- --ignored --nocapture`
    #[test]
    #[ignore = "calls the configured AI endpoint"]
    fn live_background_jobs_answer_in_character() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let event = idle_event("working (main.rs - Visual Studio Code)", &["stretch".into()]).expect("a valid idle event");
        println!("idle: {event:?}");
        let (line, emotion) = proactive_line("The user just started playing Valorant. Hype them up.", "happy").expect("a line");
        println!("proactive: [{emotion}] {line}");
        assert!(!line.contains("[EMOTION"));
    }

    #[test]
    fn json_is_dug_out_of_chatty_replies() {
        assert_eq!(json_body("Sure! ```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(json_body("{\"found\": false}"), "{\"found\": false}");
    }
}
