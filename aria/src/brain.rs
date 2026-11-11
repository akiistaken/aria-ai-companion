//! The brain: OpenAI-compatible chat with tool calls, offline fallback, persona flavour,
//! and the media/dropped-file analyses.

use serde_json::{json, Value};
use std::sync::Mutex;

pub struct Reply {
    pub text: String,
    pub emotion: String,
    pub logs: Vec<String>,
}

impl Reply {
    pub fn new(text: impl Into<String>, emotion: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            emotion: emotion.into(),
            logs: Vec::new(),
        }
    }

    pub fn logs(mut self, logs: Vec<String>) -> Self {
        self.logs = logs;
        self
    }
}

/// One attached image or file the user dropped.
// TODO(wire-up): attachments are parsed by the LLM path but not surfaced in the UI yet.
#[allow(dead_code)]
pub enum Attachment {
    Path(String),
    Text(String),
}

pub struct Brain {
    history: Mutex<Vec<Value>>,
}

static BRAIN: std::sync::OnceLock<Brain> = std::sync::OnceLock::new();

pub fn brain() -> &'static Brain {
    BRAIN.get_or_init(|| Brain {
        history: Mutex::new(Vec::new()),
    })
}

impl Brain {
    pub fn reset_memory(&self) {
        self.history.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    fn history_snapshot(&self) -> Vec<Value> {
        let h = self.history.lock().unwrap_or_else(|e| e.into_inner());
        h.iter().rev().take(10).rev().cloned().collect()
    }

    fn push_history(&self, entries: Vec<Value>) {
        let mut h = self.history.lock().unwrap_or_else(|e| e.into_inner());
        h.extend(entries);
        if h.len() > 20 {
            let cut = h.len() - 20;
            h.drain(..cut);
        }
    }

    /// Main conversational entry point. `move_cb` fires when the model moves the mascot.
    pub fn chat(
        &self,
        user_message: &str,
        include_screen: bool,
        move_cb: &dyn Fn(&str, Option<&str>),
    ) -> Reply {
        if let Some(reply) = persona_switch_command(user_message) {
            return reply;
        }
        crate::mood::note_user_text(user_message);

        let api_key = crate::config::api_key();
        if api_key.is_empty() {
            if let Some((text, emotion)) = ollama_fallback(
                user_message,
                &format!(
                    "{}\n{}",
                    crate::config::system_instruction(),
                    crate::memory::context_block()
                ),
            ) {
                return Reply::new(text, emotion).logs(vec![
                    "Answered locally via Ollama (offline mode)".into(),
                ]);
            }
            return self.offline_fallback(user_message, include_screen, move_cb);
        }

        let mut logs = Vec::new();
        let system_prompt = format!(
            "{}\n{}",
            crate::config::system_instruction(),
            crate::memory::context_block()
        );

        let need_screen = include_screen || should_capture_screen(user_message);
        let user_content = if need_screen {
            match crate::win32::capture_screen_jpeg_b64(1280) {
                Some(b64) => {
                    logs.push("Captured live desktop screenshot".into());
                    json!([
                        { "type": "text", "text": user_message },
                        { "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{b64}") } }
                    ])
                }
                None => json!(user_message),
            }
        } else {
            json!(user_message)
        };

        let mut messages = vec![json!({ "role": "system", "content": system_prompt })];
        messages.extend(self.history_snapshot());
        messages.push(json!({ "role": "user", "content": user_content }));

        let mut raw_reply = String::new();
        for _round in 0..5 {
            let body = json!({
                "model": crate::config::model(),
                "messages": messages,
                "tools": crate::tools::openai_tools(),
            });

            let response = match post_chat(&body) {
                Ok(v) => v,
                Err(e) => {
                    let err = e.to_lowercase();
                    let vision_rejected = ["image", "vision", "multimodal", "unsupported", "invalid image"]
                        .iter()
                        .any(|k| err.contains(k));
                    if vision_rejected && need_screen {
                        log::warn!("model rejected image payload, retrying text-only: {e}");
                        if let Some(last) = messages.last_mut() {
                            *last = json!({
                                "role": "user",
                                "content": format!("{user_message} (Note: the desktop screenshot was captured, but this model is text-only)")
                            });
                        }
                        match post_chat(&body) {
                            Ok(v) => v,
                            Err(e2) => {
                                return self.api_failure(&e2, user_message, logs);
                            }
                        }
                    } else {
                        return self.api_failure(&e, user_message, logs);
                    }
                }
            };

            let Some(choice) = response
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
                .cloned()
            else {
                return self.api_failure("empty response from the model", user_message, logs_finish(logs));
            };
            let message = choice.get("message").cloned().unwrap_or_else(|| json!({}));

            let mut assistant = serde_json::Map::new();
            assistant.insert("role".into(), json!("assistant"));
            if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                if !content.is_empty() {
                    assistant.insert("content".into(), json!(content));
                    raw_reply = content.to_string();
                }
            }
            let tool_calls = message
                .get("tool_calls")
                .and_then(|t| t.as_array())
                .cloned()
                .unwrap_or_default();
            if !tool_calls.is_empty() {
                assistant.insert("tool_calls".into(), json!(tool_calls));
            }
            messages.push(Value::Object(assistant));

            if tool_calls.is_empty() {
                break;
            }

            for call in tool_calls {
                let id = call.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let function = call.get("function").cloned().unwrap_or_else(|| json!({}));
                let name = function
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let args_text = function
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let args: Value = serde_json::from_str(args_text).unwrap_or_else(|_| json!({}));
                logs.push(format!("{name}({})", compact_json(&args)));
                let result = crate::tools::execute_tool(&name, &args, move_cb);
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": result.to_string(),
                }));
            }
        }

        self.push_history(vec![
            json!({ "role": "user", "content": user_message }),
            json!({ "role": "assistant", "content": raw_reply }),
        ]);

        let (clean, emotion) = parse_emotion(&raw_reply, "talking");
        if let Some(mood) = extract_mood(&raw_reply) {
            crate::mood::set_direct_mood(&mood, "Aria decided her mood in conversation");
            logs.push(format!("Mood shift: Aria adopted {} mood", capitalize(&mood)));
        }
        award_emotion_exp(&emotion);

        let mut logs = logs;
        self.extract_and_remember(user_message, &logs);
        logs.shrink_to_fit();
        Reply {
            text: clean,
            emotion,
            logs,
        }
    }

    fn api_failure(&self, err: &str, user_message: &str, logs: Vec<String>) -> Reply {
        log::error!("AI API chat error: {err}");
        if let Some((text, emotion)) = ollama_fallback(user_message, &crate::config::system_instruction()) {
            let mut logs = logs;
            logs.push("Answered locally via Ollama (network failover)".into());
            return Reply { text, emotion, logs };
        }
        Reply::new(
            crate::config::flav(&format!(
                "Something went wrong with my neural link ({})! Check your API key in the Control Center!",
                crate::util::truncate(err, 120)
            )),
            "annoyed",
        )
        .logs(logs)
    }

    /// Autonomously extracts and saves meaningful facts from the user's message.
    fn extract_and_remember(&self, user_message: &str, tool_logs: &[String]) {
        // If the model already called remember_user_fact this turn, don't duplicate it.
        if tool_logs.iter().any(|l| l.contains("remember_user_fact")) {
            return;
        }
        let msg = user_message.trim();
        let mut extracted: Option<(String, &'static str)> = None;

        let name_re = regex::Regex::new(r"(?i)\b(?:my name is|call me|i am|i'm)\s+([A-Z][a-zA-Z0-9_]{1,20})\b").unwrap();
        if let Some(cap) = name_re.captures(msg) {
            let candidate = cap[1].to_lowercase();
            let stop = [
                "here", "sorry", "back", "tired", "busy", "fine", "ready", "playing", "working",
                "saying", "asking", "just", "now", "not", "also", "so", "very", "a", "an",
            ];
            if !candidate.ends_with("ing") && !stop.iter().any(|w| candidate.contains(w)) {
                extracted = Some((format!("User's name is {}", &cap[1]), "personal"));
            }
        }

        if extracted.is_none() {
            let re = regex::Regex::new(r"(?i)\bmy favorite\s+([a-zA-Z0-9_\s]{2,25})\s+is\s+([^.!?\n]{2,40})").unwrap();
            if let Some(cap) = re.captures(msg) {
                extracted = Some((
                    format!("User's favorite {} is {}", cap[1].trim(), cap[2].trim()),
                    "favorites",
                ));
            }
        }

        if extracted.is_none() {
            let re = regex::Regex::new(r"(?i)\bi\s+(?:really\s+)?(?:love|like|enjoy|prefer)\s+([^.!?\n]{3,50})").unwrap();
            if let Some(cap) = re.captures(msg) {
                let low = cap[1].to_lowercase();
                let skip = ["you", "it", "that", "this", "how", "what", "to talk", "hearing"];
                if !skip.iter().any(|w| low.contains(w)) {
                    extracted = Some((format!("User likes/enjoys: {}", cap[1].trim()), "preference"));
                }
            }
        }

        if extracted.is_none() {
            let re = regex::Regex::new(r"(?i)\bi\s+(?:really\s+)?(?:hate|dislike|can't stand)\s+([^.!?\n]{3,50})").unwrap();
            if let Some(cap) = re.captures(msg) {
                let low = cap[1].to_lowercase();
                if !["you", "it", "that", "this"].iter().any(|w| low.contains(w)) {
                    extracted = Some((format!("User dislikes: {}", cap[1].trim()), "preference"));
                }
            }
        }

        if extracted.is_none() {
            let re = regex::Regex::new(r"(?i)\bi(?:'m|\s+am)\s+(?:working on|building|coding|developing|writing)\s+([^.!?\n]{3,55})").unwrap();
            if let Some(cap) = re.captures(msg) {
                extracted = Some((format!("User is working on: {}", cap[1].trim()), "project"));
            }
        }

        if extracted.is_none() {
            let re = regex::Regex::new(r"(?i)\bi\s+(?:often\s+)?(?:play|main)\s+([a-zA-Z0-9_\s]{2,30})").unwrap();
            if let Some(cap) = re.captures(msg) {
                let low = cap[1].to_lowercase();
                if !["with", "around", "here", "it", "that"].iter().any(|w| low.contains(w)) {
                    extracted = Some((format!("User plays/mains: {}", cap[1].trim()), "gaming"));
                }
            }
        }

        if let Some((fact, category)) = extracted {
            let (saved, _) = crate::memory::add_fact(&fact, category);
            if saved {
                log::info!("remembered: {fact}");
            }
        }
    }
}

fn logs_finish(logs: Vec<String>) -> Vec<String> {
    logs
}

fn compact_json(value: &Value) -> String {
    let s = value.to_string();
    if s == "{}" {
        String::new()
    } else {
        s
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn award_emotion_exp(emotion: &str) {
    let key = match emotion {
        "happy" | "idle" => ("dere", 3),
        "annoyed" => ("tsun", 3),
        "dance" | "walking" => ("hype", 3),
        "groove" => ("groove", 4),
        "sit" | "sleepy" => ("devotion", 3),
        _ => ("dere", 1),
    };
    crate::memory::adjust_emotion(key.0, key.1);
}

fn should_capture_screen(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    [
        "screen",
        "looking at",
        "what am i",
        "what's on",
        "whats on",
        "see my",
        "look at",
        "this window",
        "watching",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

/// POSTs a chat completion and returns the decoded JSON.
pub fn post_chat(body: &Value) -> Result<Value, String> {
    let url = format!("{}/chat/completions", crate::config::base_url().trim_end_matches('/'));
    let mut response = ureq::post(&url)
        .header("Authorization", &format!("Bearer {}", crate::config::api_key()))
        .header("Content-Type", "application/json")
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(90)))
        .build()
        .send_json(body)
        .map_err(|e| e.to_string())?;
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("{e}: {text}"))
}

pub fn message_content(response: &Value) -> Option<String> {
    response
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?
        .as_str()
        .map(str::to_string)
}

/// Local Ollama fallback (127.0.0.1:11434) used when no API key is configured.
fn ollama_fallback(prompt: &str, system_prompt: &str) -> Option<(String, String)> {
    let tags = ureq::get("http://127.0.0.1:11434/api/tags")
        .config()
        .timeout_global(Some(std::time::Duration::from_millis(1200)))
        .build()
        .call()
        .ok()?;
    let data: Value = tags.into_body().read_json().ok()?;
    let model = data.get("models")?.as_array()?.first()?.get("name")?.as_str()?.to_string();

    let response = ureq::post("http://127.0.0.1:11434/api/generate")
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(8)))
        .build()
        .send_json(json!({
            "model": model,
            "prompt": prompt,
            "system": system_prompt,
            "stream": false,
        }))
        .ok()?;
    let out: Value = response.into_body().read_json().ok()?;
    let raw = out.get("response")?.as_str()?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    Some(parse_emotion(&raw, "talking"))
}

static EMOTION_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
static MOOD_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

fn emotion_regex() -> &'static regex::Regex {
    EMOTION_RE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)\[EMOTION:\s*(idle|talking|annoyed|walking|happy|dizzy|sleepy|dance|sit|groove)\]",
        )
        .unwrap()
    })
}

fn mood_regex() -> &'static regex::Regex {
    MOOD_RE.get_or_init(|| {
        regex::Regex::new(r"(?i)\[MOOD:\s*(tsundere|yandere|tomboy|bakadere|kuudere|genki|meganekko)\]")
            .unwrap()
    })
}

pub fn parse_emotion(raw: &str, default_emotion: &str) -> (String, String) {
    if raw.trim().is_empty() {
        return (
            crate::config::flav("I didn't have anything to say to that!"),
            "annoyed".into(),
        );
    }
    let emotion = emotion_regex()
        .captures(raw)
        .map(|c| c[1].to_lowercase())
        .unwrap_or_else(|| default_emotion.to_string());
    let cleaned = mood_regex().replace_all(raw, "");
    let cleaned = emotion_regex().replace_all(&cleaned, "");
    let text = cleaned.trim().to_string();
    (
        if text.is_empty() {
            raw.trim().to_string()
        } else {
            text
        },
        emotion,
    )
}

pub fn extract_mood(raw: &str) -> Option<String> {
    mood_regex()
        .captures(raw)
        .map(|c| c[1].to_lowercase())
}

/// Handles direct "switch to yandere mode" requests with full agency in the reply.
fn persona_switch_command(message: &str) -> Option<Reply> {
    let lower = message.to_lowercase();
    let lower = lower.trim();
    let table: &[(&str, &[&str])] = &[
        ("yandere", &["switch to yandere", "yandere mode", "be yandere", "be a yandere", "persona yandere"]),
        ("tomboy", &["switch to tomboy", "tomboy mode", "be tomboy", "be a tomboy", "persona tomboy"]),
        ("tsundere", &["switch to tsundere", "tsundere mode", "be tsundere", "be a tsundere", "persona tsundere"]),
        ("bakadere", &["switch to bakadere", "bakadere mode", "be bakadere", "be a bakadere", "persona bakadere", "be goofy", "be clumsy"]),
        ("kuudere", &["switch to kuudere", "kuudere mode", "be kuudere", "be a kuudere", "persona kuudere", "be cold", "be quiet"]),
        ("genki", &["switch to genki", "genki mode", "be genki", "genki girl", "be a genki girl", "persona genki", "hyper mode"]),
        ("meganekko", &["switch to meganekko", "meganekko mode", "be meganekko", "be a meganekko", "persona meganekko", "glasses mode", "be smart"]),
    ];

    for (key, phrases) in table {
        if lower == *key || phrases.iter().any(|p| lower.contains(p)) {
            crate::mood::set_direct_mood(key, "user requested this side");
            let line = match *key {
                "tsundere" => "You want me acting all tsundere? Hmph, I decide my own mood, baka! But I'll keep you on your toes!",
                "yandere" => "Hehe~ Calling for my sweet, possessive side? You have my full undivided attention now, darling...",
                "tomboy" => "Calling for Tomboy energy? Let's go! I'm hyped and ready to crush whatever we're doing!",
                "bakadere" => "Ehehe~! You called for Bakadere mode? Yay! *trips over air* Wait, what were we doing again? Love you lots anyway!",
                "kuudere" => "...You requested kuudere. Very well. Observation active. Emotional variance minimized, but focus maintained.",
                "genki" => "YAAAAY! GENKI POWER ACTIVATED!! Let's do this! Let's conquer the world together!",
                _ => "*adjusts glasses carefully* Meganekko mode engaged. Analysis parameters initialized. How may my intellect serve your goals today?",
            };
            let emotion = match *key {
                "yandere" | "tomboy" => "happy",
                "bakadere" => "dizzy",
                "kuudere" => "idle",
                "genki" => "dance",
                _ => "talking",
            };
            let mut reply = Reply::new(
                crate::config::flavor_with(line, Some(key), 0.35),
                emotion,
            );
            reply
                .logs
                .push(format!("Autonomous mood: Aria adopted {} mood", capitalize(key)));
            return Some(reply);
        }
    }
    None
}

/// Offline mode: still fully functional PC control, just with scripted dialogue.
impl Brain {
    fn offline_fallback(
        &self,
        user_message: &str,
        include_screen: bool,
        move_cb: &dyn Fn(&str, Option<&str>),
    ) -> Reply {
        crate::offline::answer(user_message, include_screen, move_cb)
    }
}

/// "What do you hear?" â€” loopback capture plus optional model description.
pub fn listen_to_desktop_audio(seconds: f64, question: &str) -> Value {
    let activity = crate::audio::audio_activity(240);
    let (samples, levels) = crate::audio::capture_desktop_audio(seconds);
    let audible_apps: Vec<String> = activity.apps.iter().map(|a| a.name.clone()).collect();

    let mut result = json!({
        "status": "success",
        "audible_apps": audible_apps,
        "levels": levels,
    });

    if samples.is_empty() {
        if let Some(obj) = result.as_object_mut() {
            obj.insert("status".into(), json!("error"));
            obj.insert(
                "error".into(),
                json!(levels.error.clone().unwrap_or_else(|| "Could not capture desktop audio".into())),
            );
        }
        return result;
    }
    if levels.silent {
        if let Some(obj) = result.as_object_mut() {
            obj.insert(
                "heard".into(),
                json!("Silence â€” nothing is playing through the speakers right now."),
            );
        }
        return result;
    }

    let api_key = crate::config::api_key();
    if api_key.is_empty() {
        let apps = if audible_apps.is_empty() {
            "an unknown app".to_string()
        } else {
            audible_apps.join(", ")
        };
        if let Some(obj) = result.as_object_mut() {
            obj.insert(
                "heard".into(),
                json!(format!(
                    "Audio is playing (level {} dBFS) from {apps}. Add an API key so I can tell you what it actually is.",
                    levels.dbfs
                )),
            );
        }
        return result;
    }

    let wav = crate::audio::wav_bytes(&samples, 16_000);
    let prompt = if question.trim().is_empty() {
        "This is a clip of the user's desktop audio. Briefly say what it is: if music, the likely song/artist \
         (only if you recognise it), genre and mood; if speech or video, what is being said; if a game, what it \
         sounds like. Two short sentences."
            .to_string()
    } else {
        question.trim().to_string()
    };

    let body = json!({
        "model": crate::config::model(),
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": prompt },
                { "type": "input_audio", "input_audio": {
                    "data": crate::win32::base64_encode(&wav), "format": "wav" } }
            ]
        }]
    });

    let heard = match post_chat(&body) {
        Ok(response) => message_content(&response)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "I heard audio but couldn't make out what it was.".into()),
        Err(e) => format!(
            "Audio is playing ({} dBFS) but my audio analysis failed: {}",
            levels.dbfs,
            crate::util::truncate(&e, 100)
        ),
    };
    if let Some(obj) = result.as_object_mut() {
        obj.insert("heard".into(), json!(heard));
    }
    result
}

/// Dropped file / image / text analysis.
// TODO(wire-up): drag-and-drop of files is not wired to a window yet.
#[allow(dead_code)]
pub fn analyze_dropped(path_or_text: &str) -> Reply {
    let path = std::path::Path::new(path_or_text);
    if path.is_file() {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if ["png", "jpg", "jpeg", "webp", "bmp", "gif"].contains(&ext.as_str()) {
            if !crate::config::api_key().is_empty() {
                if let Ok(bytes) = std::fs::read(path) {
                    let mime = if ext == "png" { "image/png" } else { "image/jpeg" };
                    let body = json!({
                        "model": crate::config::model(),
                        "messages": [
                            { "role": "system", "content": format!("{}\n{}", crate::config::system_instruction(), crate::memory::context_block()) },
                            { "role": "user", "content": [
                                { "type": "text", "text": format!("The user just dragged and dropped this image file ('{name}') onto your avatar. Inspect it and react in character!") },
                                { "type": "image_url", "image_url": { "url": format!("data:{mime};base64,{}", crate::win32::base64_encode(&bytes)) } }
                            ]}
                        ]
                    });
                    if let Ok(response) = post_chat(&body) {
                        if let Some(content) = message_content(&response) {
                            let (text, emotion) = parse_emotion(&content, "talking");
                            return Reply::new(text, emotion)
                                .logs(vec![format!("Analyzed dropped image: {name}")]);
                        }
                    }
                }
            }
            if let Ok(img) = image::open(path) {
                return Reply::new(
                    crate::config::flav(&format!(
                        "You dropped the image `{name}` ({}x{}) onto me! Set your API key if you want me to describe what's inside it.",
                        img.width(),
                        img.height()
                    )),
                    "happy",
                )
                .logs(vec![format!("Dropped image: {name}")]);
            }
            return Reply::new(
                crate::config::flav(&format!("I couldn't open that image ({name})!")),
                "annoyed",
            );
        }

        let raw = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(_) => {
                let kb = std::fs::metadata(path).map(|m| m.len() / 1024).unwrap_or(0);
                return Reply::new(
                    crate::config::flav(&format!(
                        "You dropped `{name}` on me, but it's a binary file ({kb} KB)!"
                    )),
                    "annoyed",
                )
                .logs(vec![format!("Dropped file: {name}")]);
            }
        };

        if !crate::config::api_key().is_empty() {
            let body = json!({
                "model": crate::config::model(),
                "messages": [
                    { "role": "system", "content": format!("{}\n{}", crate::config::system_instruction(), crate::memory::context_block()) },
                    { "role": "user", "content": format!(
                        "The user dragged and dropped the file '{name}' onto your avatar. Summarize it or spot any bugs concisely in your active persona:\n\n{}",
                        crate::util::truncate(&raw, 3500)
                    )}
                ]
            });
            if let Ok(response) = post_chat(&body) {
                if let Some(content) = message_content(&response) {
                    let (text, emotion) = parse_emotion(&content, "talking");
                    return Reply::new(text, emotion).logs(vec![format!("Analyzed dropped file: {name}")]);
                }
            }
        }

        let lines: Vec<&str> = raw.lines().collect();
        let funcs = lines
            .iter()
            .filter(|ln| {
                let t = ln.trim_start();
                t.starts_with("def ") || t.starts_with("async def ") || t.starts_with("function ") || t.starts_with("class ") || t.starts_with("fn ")
            })
            .count();
        let errs: Vec<&str> = lines
            .iter()
            .filter(|ln| {
                ["Error:", "Exception:", "Traceback", "TODO"]
                    .iter()
                    .any(|k| ln.contains(k))
            })
            .map(|ln| ln.trim())
            .collect();
        let note = match errs.last() {
            Some(e) => format!(" Found an issue/note: `{}`!", crate::util::truncate(e, 70)),
            None => " Looks clean with no obvious error lines!".to_string(),
        };
        return Reply::new(
            crate::config::flav(&format!(
                "I inspected your dropped file `{name}` ({} lines, {funcs} functions/classes).{note}",
                lines.len()
            )),
            "talking",
        )
        .logs(vec![format!("Inspected file: {name} ({} lines)", lines.len())]);
    }

    brain().chat(
        &format!("I just dropped this onto you: {}", crate::util::truncate(path_or_text, 500)),
        false,
        &|_, _| {},
    )
}

/// Clipboard traceback diagnosis.
pub fn analyze_clipboard(clip_text: &str) -> Reply {
    let lines: Vec<&str> = clip_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let last = lines
        .last()
        .map(|s| s.to_string())
        .unwrap_or_else(|| crate::util::truncate(clip_text, 120));

    if !crate::config::api_key().is_empty() {
        let body = json!({
            "model": crate::config::model(),
            "messages": [
                { "role": "system", "content": crate::config::system_instruction() },
                { "role": "user", "content": format!(
                    "The user just copied this error/traceback to their clipboard. Give a short 2-sentence explanation of the bug and how to fix it in your active persona:\n\n{}",
                    crate::util::truncate(clip_text, 2500)
                )}
            ]
        });
        if let Ok(response) = post_chat(&body) {
            if let Some(content) = message_content(&response) {
                let (text, emotion) = parse_emotion(&content, "talking");
                return Reply::new(text, emotion)
                    .logs(vec![format!("Clipboard bug diagnosis: {}", crate::util::truncate(&last, 48))]);
            }
        }
    }

    Reply::new(
        crate::config::flav(&format!(
            "I noticed you just copied an error to your clipboard (`{}`)! Check the variable types or imports on that line!",
            crate::util::truncate(&last, 85)
        )),
        "annoyed",
    )
    .logs(vec![format!("Clipboard error caught: {}", crate::util::truncate(&last, 48))])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emotion_tags_are_stripped_and_detected() {
        let (text, emotion) = parse_emotion("[EMOTION: happy] Hello there!", "talking");
        assert_eq!(emotion, "happy");
        assert_eq!(text, "Hello there!");
    }

    #[test]
    fn mood_tags_are_extracted() {
        assert_eq!(extract_mood("sure [MOOD: yandere] ok").as_deref(), Some("yandere"));
        assert_eq!(extract_mood("nothing here"), None);
    }

    #[test]
    fn missing_tag_falls_back_to_default() {
        let (text, emotion) = parse_emotion("plain", "talking");
        assert_eq!(emotion, "talking");
        assert_eq!(text, "plain");
    }

    #[test]
    fn screen_keywords_trigger_capture() {
        assert!(should_capture_screen("what do you see on my screen?"));
        assert!(!should_capture_screen("tell me a joke"));
    }
}
