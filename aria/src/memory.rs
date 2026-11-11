//! Memory vault: affection/emotion EXP, OCEAN traits, remembered facts, to-do list.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub struct EmotionDef {
    pub key: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    pub title: &'static str,
    pub desc: &'static str,
    pub color: &'static str,
    pub gradient: (&'static str, &'static str),
    pub tiers: [&'static str; 5],
}

pub static EMOTIONS: &[EmotionDef] = &[
    EmotionDef {
        key: "dere",
        name: "Dere",
        icon: "♥",
        title: "Sweetness & Affection",
        desc: "Warmth, blushing sweetness, genuine care, and loving bonds.",
        color: "#ec4899",
        gradient: ("#ec4899", "#f43f5e"),
        tiers: [
            "Guarded Heart",
            "Shy Warmth",
            "Sweet Companion",
            "Tender Devotion",
            "Pure Deredere",
        ],
    },
    EmotionDef {
        key: "tsun",
        name: "Tsun",
        icon: "⚡",
        title: "Sass & Feisty Pride",
        desc: "Sharp-tongued wit, pouting, feisty complaints, and flustered pride.",
        color: "#38bdf8",
        gradient: ("#38bdf8", "#0284c7"),
        tiers: [
            "Aloof",
            "Classic Sassy",
            "Feisty Pouter",
            "Flustered Tsundere",
            "Empress of Tsun",
        ],
    },
    EmotionDef {
        key: "devotion",
        name: "Devotion",
        icon: "◈",
        title: "Attachment & Loyalty",
        desc: "Sweetly possessive care, deep loyalty, and late-night companion bonding.",
        color: "#f43f5e",
        gradient: ("#f43f5e", "#be123c"),
        tiers: [
            "Watchful Eye",
            "Protective Companion",
            "Sweetly Clingy",
            "Inseparable Bond",
            "Eternal Devotion",
        ],
    },
    EmotionDef {
        key: "hype",
        name: "Hype",
        icon: "▲",
        title: "Energy & Playfulness",
        desc: "Gaming hype, sporty enthusiasm, celebrations, and best-bro spirit.",
        color: "#f59e0b",
        gradient: ("#f59e0b", "#d97706"),
        tiers: [
            "Casual Chill",
            "Sporty Energy",
            "Clutch Gamer",
            "Hype Bestie",
            "Ultimate Wingman",
        ],
    },
    EmotionDef {
        key: "groove",
        name: "Groove",
        icon: "♪",
        title: "Music & Rhythm",
        desc: "Music vibe-sync, desktop dancing, and relaxed audio chilling.",
        color: "#a855f7",
        gradient: ("#a855f7", "#7c3aed"),
        tiers: [
            "Quiet Listener",
            "Head Bobber",
            "Desktop Dancer",
            "Rhythm Sync",
            "Vibe Goddess",
        ],
    },
];

pub static DEFAULT_EMOTION_EXP: &[(&str, i64)] = &[
    ("dere", 15),
    ("tsun", 25),
    ("devotion", 10),
    ("hype", 20),
    ("groove", 15),
];

pub const TRAIT_KEYS: &[(&str, &str, &str, &str)] = &[
    (
        "openness",
        "Openness",
        "Curiosity & Creativity",
        "Curiosity, creativity, appreciation for art, and adventurous exploration.",
    ),
    (
        "conscientiousness",
        "Conscientiousness",
        "Organization & Goal Focus",
        "Self-discipline, organization, dutifulness, and a strong focus on tasks & goals.",
    ),
    (
        "extraversion",
        "Extraversion",
        "Energy & Sociability",
        "Sociability, talkativeness, high energy, enthusiasm, and assertiveness.",
    ),
    (
        "agreeableness",
        "Agreeableness",
        "Empathy & Compassion",
        "Compassion, cooperativeness, trust, empathy, and loyalty toward others.",
    ),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub fact: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub added_at: String,
}

fn default_category() -> String {
    "general".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub task: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub added_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    pub affection: i64,
    pub total_interactions: i64,
    pub emotion_exp: std::collections::BTreeMap<String, i64>,
    pub traits: std::collections::BTreeMap<String, i64>,
    pub facts: Vec<Fact>,
    pub todos: Vec<Todo>,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            affection: 15,
            total_interactions: 0,
            emotion_exp: DEFAULT_EMOTION_EXP
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
            traits: [
                ("openness", 85),
                ("conscientiousness", 72),
                ("extraversion", 78),
                ("agreeableness", 74),
            ]
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect(),
            facts: Vec::new(),
            todos: Vec::new(),
        }
    }
}

static MEMORY: OnceLock<Mutex<Memory>> = OnceLock::new();

fn memory_file() -> PathBuf {
    crate::config::data_dir().join("aria_memory.json")
}

fn store() -> &'static Mutex<Memory> {
    MEMORY.get_or_init(|| {
        let mem = std::fs::read_to_string(memory_file())
            .ok()
            .and_then(|raw| serde_json::from_str::<Memory>(&raw).ok())
            .unwrap_or_default();
        Mutex::new(mem)
    })
}

pub fn load() -> Memory {
    store().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn persist(mem: &Memory) {
    if let Ok(text) = serde_json::to_string_pretty(mem) {
        let _ = std::fs::write(memory_file(), text);
    }
}

fn mutate<R>(f: impl FnOnce(&mut Memory) -> R) -> R {
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    let out = f(&mut guard);
    persist(&guard);
    out
}

/// One emotion meter. `name`, `desc` and `gradient` are catalogue data the current Control Center
/// does not display (it shows icon, title and colour), kept so the meters stay self-describing.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EmotionStat {
    pub key: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    pub title: &'static str,
    pub desc: &'static str,
    pub color: &'static str,
    pub gradient: (&'static str, &'static str),
    pub total_exp: i64,
    pub level: i64,
    pub progress: i64,
    pub tier: &'static str,
}

fn emotion_def(key: &str) -> &'static EmotionDef {
    EMOTIONS.iter().find(|e| e.key == key).unwrap_or(&EMOTIONS[0])
}

pub fn emotion_stats(key: &str) -> EmotionStat {
    let def = emotion_def(key);
    let mem = load();
    let total = *mem
        .emotion_exp
        .get(def.key)
        .unwrap_or(&DEFAULT_EMOTION_EXP[0].1);
    let level = (total / 100 + 1).max(1);
    let progress = total % 100;
    let idx = ((level - 1) as usize).min(def.tiers.len() - 1);
    EmotionStat {
        key: def.key,
        name: def.name,
        icon: def.icon,
        title: def.title,
        desc: def.desc,
        color: def.color,
        gradient: def.gradient,
        total_exp: total,
        level,
        progress,
        tier: def.tiers[idx],
    }
}

pub fn all_emotion_stats() -> Vec<EmotionStat> {
    EMOTIONS.iter().map(|e| emotion_stats(e.key)).collect()
}

pub fn adjust_emotion(key: &str, delta: i64) -> EmotionStat {
    let def = emotion_def(key).key;
    mutate(|mem| {
        let cur = *mem
            .emotion_exp
            .get(def)
            .unwrap_or(&DEFAULT_EMOTION_EXP[0].1);
        let next = (cur + delta).max(0);
        mem.emotion_exp.insert(def.to_string(), next);
        if def == "dere" {
            mem.affection = next.min(100);
        }
        mem.total_interactions += i64::from(delta >= 0);
    });
    emotion_stats(def)
}

pub fn reset_emotions() {
    mutate(|mem| {
        mem.emotion_exp = DEFAULT_EMOTION_EXP
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect();
        mem.affection = DEFAULT_EMOTION_EXP[0].1;
    });
}

pub fn affection() -> i64 {
    let mem = load();
    *mem.emotion_exp.get("dere").unwrap_or(&mem.affection)
}

pub fn adjust_affection(delta: i64) -> i64 {
    adjust_emotion("dere", delta).total_exp
}

pub fn affection_tier_label() -> (String, String) {
    let stat = emotion_stats("dere");
    let guidance = format!(
        "Your sweetness/dere emotion is Level {} ({}). Express your natural affinity accordingly \
         while Aria stays true to her active autonomous mood.",
        stat.level, stat.tier
    );
    (
        format!("{} (Lvl {})", stat.tier, stat.level),
        guidance,
    )
}

pub fn traits() -> std::collections::BTreeMap<String, i64> {
    load().traits
}

pub fn set_trait(name: &str, value: i64) -> std::collections::BTreeMap<String, i64> {
    let value = value.clamp(0, 100);
    let key = name.to_lowercase();
    if TRAIT_KEYS.iter().any(|(k, ..)| *k == key) {
        mutate(|mem| {
            mem.traits.insert(key.clone(), value);
        });
    }
    traits()
}

/// Adds a remembered fact. Returns (saved?, total facts).
pub fn add_fact(fact: &str, category: &str) -> (bool, usize) {
    let clean = fact.trim().trim_end_matches('.').trim().to_string();
    if clean.is_empty() {
        return (false, 0);
    }
    let lower = clean.to_lowercase();
    mutate(|mem| {
        for existing in &mem.facts {
            let ex = existing.fact.to_lowercase();
            if ex == lower || (lower.len() > 8 && ex.contains(&lower)) {
                return (false, mem.facts.len());
            }
        }
        mem.facts.push(Fact {
            fact: clean.clone(),
            category: category.to_string(),
            added_at: crate::util::now_stamp("%Y-%m-%d %H:%M"),
        });
        if mem.facts.len() > 100 {
            let cut = mem.facts.len() - 100;
            mem.facts.drain(..cut);
        }
        mem.affection = (mem.affection + 3).min(100);
        (true, mem.facts.len())
    })
}

pub fn facts() -> Vec<Fact> {
    load().facts
}

pub fn fact_texts() -> Vec<String> {
    load().facts.into_iter().map(|f| f.fact).collect()
}

pub fn todos() -> Vec<Todo> {
    load().todos
}

pub enum TodoAction {
    Add,
    Complete,
    Clear,
    List,
}

/// Returns (status, matched task, resulting list).
pub fn manage_todos(action: TodoAction, task: &str) -> (&'static str, Option<String>, Vec<Todo>) {
    let clean = task.trim().to_string();
    match action {
        TodoAction::Add if !clean.is_empty() => {
            let list = mutate(|mem| {
                mem.todos.push(Todo {
                    task: clean.clone(),
                    done: false,
                    added_at: crate::util::now_stamp("%H:%M"),
                });
                mem.todos.clone()
            });
            ("added", Some(clean), list)
        }
        TodoAction::Complete if !clean.is_empty() => {
            let (matched, list) = mutate(|mem| {
                let needle = clean.to_lowercase();
                let mut matched = None;
                for todo in &mut mem.todos {
                    if todo.task.to_lowercase().contains(&needle) {
                        todo.done = true;
                        matched = Some(todo.task.clone());
                        break;
                    }
                }
                mem.affection = (mem.affection + 4).min(100);
                (matched, mem.todos.clone())
            });
            (
                if matched.is_some() {
                    "completed"
                } else {
                    "not_found"
                },
                matched,
                list,
            )
        }
        TodoAction::Clear => {
            let list = mutate(|mem| {
                mem.todos.clear();
                mem.todos.clone()
            });
            ("cleared", None, list)
        }
        _ => ("success", None, todos()),
    }
}

/// Context block injected into the system prompt.
pub fn context_block() -> String {
    let mem = load();
    let mut lines = vec!["\nPSYCHOLOGICAL PERSONALITY TRAITS:".to_string()];
    for (key, name, _label, desc) in TRAIT_KEYS {
        let v = mem.traits.get(*key).copied().unwrap_or(70);
        lines.push(format!("  • {name}: {v}/100 ({desc})"));
    }
    lines.push("\nAUTONOMOUS EMOTIONAL EXP & RESONANCE BARS:".into());
    for stat in all_emotion_stats() {
        lines.push(format!(
            "  • {} {}: Level {} ({}/100 EXP) — {}",
            stat.icon, stat.title, stat.level, stat.progress, stat.tier
        ));
    }
    lines.push(
        "Aria autonomously decides her own mood and feelings. Reflect your traits and emotion EXP \
         naturally in your dialogue!"
            .into(),
    );
    if !mem.facts.is_empty() {
        lines.push(
            "AUTONOMOUS MEMORY VAULT (remembered facts & observations about the user):".into(),
        );
        for f in mem.facts.iter().rev().take(25).rev() {
            lines.push(format!("- {}", f.fact));
        }
        lines.push("Use these memories naturally to personalize your dialogue and build rapport!".into());
    }
    if !mem.todos.is_empty() {
        lines.push("USER'S CURRENT TO-DO TASKS:".into());
        for t in mem.todos.iter().rev().take(10).rev() {
            lines.push(format!("- {} {}", if t.done { "[x]" } else { "[ ]" }, t.task));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emotion_tiers_track_exp() {
        let stat = emotion_stats("tsun");
        assert!(stat.level >= 1);
        assert!(stat.progress < 100);
        assert!(!stat.tier.is_empty());
    }

    #[test]
    fn todo_lifecycle() {
        let (status, _, list) = manage_todos(TodoAction::Add, "write rust port");
        assert_eq!(status, "added");
        assert!(list.iter().any(|t| t.task == "write rust port"));
        let (status, matched, _) = manage_todos(TodoAction::Complete, "rust");
        assert_eq!(status, "completed");
        assert_eq!(matched.as_deref(), Some("write rust port"));
        manage_todos(TodoAction::Clear, "");
        assert!(todos().is_empty());
    }

    #[test]
    fn duplicate_facts_are_rejected() {
        add_fact("User's name is Aki", "personal");
        let (saved, _) = add_fact("user's name is aki", "personal");
        assert!(!saved, "case-insensitive duplicate should be rejected");
    }
}
