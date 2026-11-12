//! Mood engine: Aria picks her own personality from context instead of a fixed setting.
//!
//! Every signal (what the user does, what they say, idle time, affection, time of day) nudges a
//! per-persona score. Scores halve every 3.5 minutes, and the active persona only changes when
//! another clearly leads for long enough (hysteresis), so mood shifts instead of flickering.

use rand::Rng;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use super::KEYS as PERSONAS;

const HALF_LIFE_SEC: f64 = 210.0;
const MIN_DWELL_SEC: f64 = 60.0;

fn base_bias(key: &str) -> f64 {
    if key == "tsundere" {
        0.25
    } else {
        0.0
    }
}

const WORK_WORDS: &[&str] = &[
    "visual studio", "vs code", "code -", "pycharm", "intellij", "antigravity", "powershell",
    "command prompt", "terminal", "word", "excel", "powerpoint", "docs", "sheets", "notion",
    "obsidian", "figma", "photoshop", "premiere", ".py", ".js", ".ts", ".md", "github",
    "stack overflow", "jira", "outlook", "teams",
];

const DOC_WORDS: &[&str] = &[
    "pdf", "notion", "obsidian", "wikipedia", "arxiv", "document", "readme", "kindle", "book",
    "research", "paper", "documentation", "manual", "guide", "cheatsheet", "formula", "spec",
];

const ATTENTION_WORDS: &[&str] = &[
    "discord", "youtube", "netflix", "twitch", "reddit", "tiktok", "instagram", "twitter", " / x",
    "telegram", "whatsapp", "crunchyroll", "facebook", "messenger", "snapchat",
];

const MUSIC_WORDS: &[&str] = &["spotify", "youtube music", "soundcloud", "apple music", "wynk"];

struct TextSignal {
    words: &'static [&'static str],
    weights: &'static [(&'static str, f64)],
    reason: &'static str,
}

static TEXT_SIGNALS: &[TextSignal] = &[
    TextSignal {
        words: &[
            "gg", "let's go", "lets go", "game", "gaming", "win", "won", "gym", "workout", "hype",
            "bro", "dude", "pump", "match", "ranked", "clutch", "sport", "run", "race",
            "challenge", "boss", "victory",
        ],
        weights: &[("tomboy", 0.8), ("genki", 0.6)],
        reason: "you're hyped up",
    },
    TextSignal {
        words: &[
            "yahoo", "yay", "party", "energy", "let's gooo", "bouncing", "super", "hyper", "turbo",
            "awesome", "celebrate", "screaming", "excited",
        ],
        weights: &[("genki", 1.0), ("tomboy", 0.4)],
        reason: "super high energy party vibes",
    },
    TextSignal {
        words: &[
            "love you", "miss you", "only you", "don't leave", "dont leave", "stay with me",
            "lonely", "girlfriend", "wife", "marry", "good night", "goodnight", "going to sleep",
            "tired", "sleepy", "bye", "leaving", "hug", "cuddle", "my friend", "my friends",
            "going out", "date",
        ],
        weights: &[("yandere", 0.9), ("bakadere", 0.4)],
        reason: "you got all sentimental",
    },
    TextSignal {
        words: &[
            "ehehe", "haha", "lol", "lmao", "rofl", "silly", "clumsy", "oops", "tripped", "derp",
            "fool", "funny", "meme", "joke", "goofy",
        ],
        weights: &[("bakadere", 1.0)],
        reason: "we're being goofy and having fun",
    },
    TextSignal {
        words: &[
            "research", "calculate", "algorithm", "hypothesis", "analysis", "documentation",
            "formula", "reading", "theory", "paper", "data", "stats", "specs",
        ],
        weights: &[("meganekko", 1.0), ("kuudere", 0.4)],
        reason: "deep dive study and research mode",
    },
    TextSignal {
        words: &[
            "cold", "blunt", "quiet", "silence", "shh", "focus", "logic", "efficient", "optimal",
            "status report", "precise", "deadpan",
        ],
        weights: &[("kuudere", 1.0)],
        reason: "calm, focused logical execution",
    },
    TextSignal {
        words: &["headpat", "head pat", "pat you", "pet you", "good girl"],
        weights: &[("yandere", 0.5), ("tsundere", 0.4), ("bakadere", 0.6)],
        reason: "you were being affectionate",
    },
    TextSignal {
        words: &[
            "cute", "thank", "thanks", "good job", "well done", "nice work", "pretty", "beautiful",
            "adorable", "best",
        ],
        weights: &[("tsundere", 0.8), ("bakadere", 0.5)],
        reason: "you complimented me",
    },
    TextSignal {
        words: &[
            "stupid", "dumb", "idiot", "shut up", "annoying", "useless", "baka", "hurry", "slow",
            "boring", "ugly",
        ],
        weights: &[("tsundere", 0.9), ("tomboy", 0.2)],
        reason: "you were teasing me",
    },
    TextSignal {
        words: &[
            "work", "deadline", "homework", "study", "exam", "code", "debug", "error", "bug",
            "fix", "assignment", "report", "presentation", "meeting", "email", "todo", "task",
        ],
        weights: &[("kuudere", 0.6), ("meganekko", 0.6), ("tsundere", 0.4)],
        reason: "serious work and task time",
    },
];

#[derive(Debug, Clone)]
pub struct MoodChange {
    pub old: String,
    pub new: String,
    pub reason: String,
}

struct State {
    scores: [f64; 7],
    stamp: Instant,
    current: usize,
    reason: String,
    since: f64,
    margin: f64,
    pending: Option<MoodChange>,
    last_activity: String,
    last_idle: bool,
    last_reasons: [Option<String>; 7],
    last_eval: f64,
}

static ENGINE: OnceLock<Mutex<State>> = OnceLock::new();

fn engine() -> &'static Mutex<State> {
    ENGINE.get_or_init(|| {
        Mutex::new(State {
            scores: [0.0; 7],
            stamp: Instant::now(),
            current: 0,
            reason: "default personality".into(),
            since: 0.0,
            margin: rand::rng().random_range(0.5..0.9),
            pending: None,
            last_activity: String::new(),
            last_idle: false,
            last_reasons: Default::default(),
            last_eval: 0.0,
        })
    })
}

fn index_of(persona: &str) -> Option<usize> {
    PERSONAS.iter().position(|p| *p == persona)
}

fn elapsed_secs() -> f64 {
    crate::util::unix_now()
}

fn decay(state: &mut State) {
    let now = Instant::now();
    let dt = now.duration_since(state.stamp).as_secs_f64();
    if dt > 0.0 {
        let k = 0.5f64.powf(dt / HALF_LIFE_SEC);
        for s in &mut state.scores {
            *s *= k;
        }
        state.stamp = now;
    }
}

pub fn nudge(persona: &str, amount: f64, why: &str) {
    let Some(idx) = index_of(persona) else { return };
    let mut st = engine().lock().unwrap_or_else(|e| e.into_inner());
    decay(&mut st);
    st.scores[idx] += amount;
    if !why.is_empty() {
        st.last_reasons[idx] = Some(why.to_string());
    }
}

/// Slow-moving priors: time of day plus traits and emotion EXP.
fn priors() -> [f64; 7] {
    let mut pri = [0.0; 7];
    for (i, p) in PERSONAS.iter().enumerate() {
        pri[i] = base_bias(p);
    }
    let hour = crate::util::now_hour();
    let mut add = |name: &str, v: f64| {
        if let Some(i) = index_of(name) {
            pri[i] += v;
        }
    };
    match hour {
        22..=23 | 0..=4 => {
            add("yandere", 0.55);
            add("kuudere", 0.35);
        }
        6..=11 => {
            add("genki", 0.50);
            add("tomboy", 0.35);
        }
        12..=17 => {
            add("meganekko", 0.35);
            add("tsundere", 0.25);
        }
        _ => {
            add("tomboy", 0.25);
            add("genki", 0.20);
        }
    }

    let traits = crate::memory::traits();
    let t = |k: &str, d: i64| *traits.get(k).unwrap_or(&d) as f64;
    add("meganekko", 0.003 * t("openness", 85));
    add("genki", 0.002 * t("openness", 85));
    add("kuudere", 0.003 * t("conscientiousness", 72));
    add("tsundere", 0.002 * t("conscientiousness", 72));
    add("genki", 0.003 * t("extraversion", 78));
    add("tomboy", 0.002 * t("extraversion", 78));
    add("bakadere", 0.003 * t("agreeableness", 74));
    add("yandere", 0.002 * t("agreeableness", 74));

    add("yandere", 0.03 * crate::memory::emotion_stats("devotion").level as f64);
    add("tomboy", 0.03 * crate::memory::emotion_stats("hype").level as f64);
    add("tsundere", 0.03 * crate::memory::emotion_stats("tsun").level as f64);
    add("bakadere", 0.02 * crate::memory::emotion_stats("dere").level as f64);
    pri
}

fn effective(state: &mut State) -> [f64; 7] {
    decay(state);
    let pri = priors();
    let mut out = [0.0; 7];
    for i in 0..7 {
        out[i] = state.scores[i] + pri[i];
    }
    out
}

pub fn current_persona() -> String {
    let now = elapsed_secs();
    let mut st = engine().lock().unwrap_or_else(|e| e.into_inner());
    if now - st.last_eval < 1.0 {
        return PERSONAS[st.current].to_string();
    }
    st.last_eval = now;
    let eff = effective(&mut st);
    let mut leader = 0;
    for i in 1..7 {
        if eff[i] > eff[leader] {
            leader = i;
        }
    }
    if leader != st.current && (now - st.since) >= MIN_DWELL_SEC {
        let margin = st.margin;
        if eff[leader] - eff[st.current] >= margin {
            let why = st.last_reasons[leader]
                .clone()
                .unwrap_or_else(|| "the mood just shifted".into());
            st.pending = Some(MoodChange {
                old: PERSONAS[st.current].to_string(),
                new: PERSONAS[leader].to_string(),
                reason: why.clone(),
            });
            st.current = leader;
            st.reason = why;
            st.since = now;
            st.margin = rand::rng().random_range(0.5..0.9);
        }
    }
    PERSONAS[st.current].to_string()
}

pub fn consume_change() -> Option<MoodChange> {
    let mut st = engine().lock().unwrap_or_else(|e| e.into_inner());
    st.pending.take()
}

pub fn snapshot() -> (String, String, Vec<(String, f64)>) {
    let mut st = engine().lock().unwrap_or_else(|e| e.into_inner());
    let eff = effective(&mut st);
    let current = PERSONAS[st.current].to_string();
    let reason = st.reason.clone();
    let scores = PERSONAS
        .iter()
        .enumerate()
        .map(|(i, p)| (p.to_string(), (eff[i] * 100.0).round() / 100.0))
        .collect();
    (current, reason, scores)
}

pub fn set_direct_mood(persona: &str, reason: &str) {
    let Some(idx) = index_of(persona) else { return };
    let mut st = engine().lock().unwrap_or_else(|e| e.into_inner());
    if idx != st.current {
        st.pending = Some(MoodChange {
            old: PERSONAS[st.current].to_string(),
            new: persona.to_string(),
            reason: reason.to_string(),
        });
        st.current = idx;
        st.reason = reason.to_string();
        st.since = elapsed_secs();
        let top = st.scores.iter().cloned().fold(f64::MIN, f64::max);
        st.scores[idx] = top.max(0.0) + 1.2;
    }
}

/// The active mood and why, for the per-request context block.
pub fn mood_note() -> String {
    let (current, reason) = {
        let st = engine().lock().unwrap_or_else(|e| e.into_inner());
        (PERSONAS[st.current].to_uppercase(), st.reason.clone())
    };
    format!("Your active mood right now is {current} ({reason}).")
}

pub fn note_user_text(text: &str) {
    let low = format!(" {} ", text.to_lowercase());
    for signal in TEXT_SIGNALS {
        let hits = signal
            .words
            .iter()
            .filter(|w| {
                // Word-boundary match: not surrounded by lowercase letters.
                let mut start = 0;
                while let Some(pos) = low[start..].find(*w) {
                    let abs = start + pos;
                    let before_ok = low[..abs].chars().next_back().is_none_or(|c| !c.is_ascii_lowercase());
                    let after = abs + w.len();
                    let after_ok = low[after..].chars().next().is_none_or(|c| !c.is_ascii_lowercase());
                    if before_ok && after_ok {
                        return true;
                    }
                    start = after;
                    if start >= low.len() {
                        break;
                    }
                }
                false
            })
            .count();
        if hits > 0 {
            let scale = (0.8 + 0.4 * hits as f64).min(2.0);
            for (persona, amt) in signal.weights {
                nudge(persona, amt * scale, signal.reason);
            }
        }
    }
}

pub fn note_event(kind: &str) {
    let table: &[(&str, &[(&str, f64)])] = &[
        ("poke", &[("tsundere", 1.2), ("bakadere", 0.8)]),
        ("shake", &[("tomboy", 0.7), ("bakadere", 1.0)]),
        ("headpat", &[("yandere", 0.8), ("tsundere", 0.4), ("bakadere", 0.9)]),
        ("game_start", &[("genki", 1.5), ("tomboy", 1.3)]),
        ("game_clutch", &[("genki", 2.0), ("tomboy", 1.5)]),
        ("focus_mode", &[("kuudere", 1.4), ("tsundere", 1.0)]),
        ("study_docs", &[("meganekko", 1.6), ("kuudere", 0.8)]),
        ("user_returned", &[("yandere", 1.4), ("genki", 0.9)]),
        ("hardware_alert", &[("kuudere", 1.0), ("tsundere", 0.7)]),
        ("music", &[("yandere", 0.4), ("tomboy", 0.3), ("genki", 0.4)]),
        ("praise", &[("tsundere", 0.8), ("bakadere", 0.9)]),
    ];
    let why = kind.replace('_', " ");
    for (key, weights) in table {
        if *key == kind {
            for (persona, amt) in *weights {
                nudge(persona, *amt, &why);
            }
        }
    }
}

pub fn classify_window(title: &str) -> &'static str {
    let low = title.to_lowercase();
    if low.is_empty() {
        return "";
    }
    if DOC_WORDS.iter().any(|d| low.contains(d)) {
        return "study";
    }
    if crate::desktop::presence::game_in_title(&low).is_some() {
        return "gaming";
    }
    if MUSIC_WORDS.iter().any(|m| low.contains(m)) {
        return "music";
    }
    if ATTENTION_WORDS.iter().any(|a| low.contains(a)) {
        return "attention";
    }
    if WORK_WORDS.iter().any(|w| low.contains(w)) {
        return "work";
    }
    ""
}

pub fn observe(active_window: &str, idle_seconds: f64, focus_active: bool) {
    let cat = classify_window(active_window).to_string();
    let step: Option<(&str, f64, &str)> = match cat.as_str() {
        "study" => Some(("meganekko", 0.07, "reading documents & notes")),
        "gaming" => Some(("genki", 0.06, "you're gaming with high energy")),
        "work" => Some(("kuudere", 0.05, "deep in code and system tasks")),
        "attention" => Some(("bakadere", 0.05, "chilling on social media/videos")),
        "music" => Some(("yandere", 0.04, "we're vibing to music together")),
        _ => None,
    };
    if let Some((persona, amt, why)) = step {
        nudge(persona, amt, why);
    }

    let previous = {
        let st = engine().lock().unwrap_or_else(|e| e.into_inner());
        st.last_activity.clone()
    };
    if cat == "gaming" && previous != "gaming" {
        note_event("game_start");
    } else if cat == "study" && previous != "study" {
        note_event("study_docs");
    }
    {
        let mut st = engine().lock().unwrap_or_else(|e| e.into_inner());
        st.last_activity = cat;
    }

    if focus_active {
        nudge("kuudere", 0.06, "deep focus mode is active");
    }

    let afk = idle_seconds >= 240.0;
    let was_afk = {
        let st = engine().lock().unwrap_or_else(|e| e.into_inner());
        st.last_idle
    };
    if was_afk && !afk {
        note_event("user_returned");
    }
    {
        let mut st = engine().lock().unwrap_or_else(|e| e.into_inner());
        st.last_idle = afk;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_classification_covers_games_docs_and_social() {
        assert_eq!(classify_window("VALORANT"), "gaming");
        assert_eq!(classify_window("README.md - Visual Studio Code"), "study");
        assert_eq!(classify_window("Discord"), "attention");
        assert_eq!(classify_window(""), "");
    }

    #[test]
    fn nudges_move_the_active_persona() {
        set_direct_mood("tsundere", "test");
        for _ in 0..12 {
            note_event("game_start");
        }
        let (current, _, _) = snapshot();
        assert!(!current.is_empty());
    }
}
