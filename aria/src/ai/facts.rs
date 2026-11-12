//! Picks obvious personal facts out of what the user says ("my name is…", "I love…"), as a
//! safety net for when the model does not call `remember_user_fact` itself.

use std::sync::OnceLock;

use regex::Regex;

struct Rule {
    pattern: &'static str,
    category: &'static str,
    /// Builds the fact from the captures, or `None` to skip this match.
    fact: fn(&regex::Captures) -> Option<String>,
}

fn skip_if_contains(text: &str, words: &[&str]) -> bool {
    let low = text.to_lowercase();
    words.iter().any(|w| low.contains(w))
}

const RULES: &[Rule] = &[
    Rule {
        pattern: r"(?i)\b(?:my name is|call me|i am|i'm)\s+([A-Z][a-zA-Z0-9_]{1,20})\b",
        category: "personal",
        fact: |c| {
            let name = c[1].to_lowercase();
            let stop = ["here", "sorry", "back", "tired", "busy", "fine", "ready", "just", "now", "not", "also", "very"];
            (!name.ends_with("ing") && !stop.contains(&name.as_str())).then(|| format!("User's name is {}", &c[1]))
        },
    },
    Rule {
        pattern: r"(?i)\bmy favou?rite\s+([a-zA-Z0-9_\s]{2,25})\s+is\s+([^.!?\n]{2,40})",
        category: "favorites",
        fact: |c| Some(format!("User's favorite {} is {}", c[1].trim(), c[2].trim())),
    },
    Rule {
        pattern: r"(?i)\bi\s+(?:really\s+)?(?:love|like|enjoy|prefer)\s+([^.!?\n]{3,50})",
        category: "preference",
        fact: |c| {
            (!skip_if_contains(&c[1], &["you", "it", "that", "this", "how", "what", "to talk", "hearing"]))
                .then(|| format!("User likes/enjoys: {}", c[1].trim()))
        },
    },
    Rule {
        pattern: r"(?i)\bi\s+(?:really\s+)?(?:hate|dislike|can't stand)\s+([^.!?\n]{3,50})",
        category: "preference",
        fact: |c| {
            (!skip_if_contains(&c[1], &["you", "it", "that", "this"])).then(|| format!("User dislikes: {}", c[1].trim()))
        },
    },
    Rule {
        pattern: r"(?i)\bi(?:'m|\s+am)\s+(?:working on|building|coding|developing|writing)\s+([^.!?\n]{3,55})",
        category: "project",
        fact: |c| Some(format!("User is working on: {}", c[1].trim())),
    },
    Rule {
        pattern: r"(?i)\bi\s+(?:often\s+)?(?:play|main)\s+([a-zA-Z0-9_\s]{2,30})",
        category: "gaming",
        fact: |c| {
            (!skip_if_contains(&c[1], &["with", "around", "here", "it", "that"]))
                .then(|| format!("User plays/mains: {}", c[1].trim()))
        },
    },
];

fn compiled() -> &'static [Regex] {
    static RES: OnceLock<Vec<Regex>> = OnceLock::new();
    RES.get_or_init(|| RULES.iter().map(|r| Regex::new(r.pattern).expect("valid regex")).collect())
}

/// The first fact `message` states, with its category.
pub fn extract(message: &str) -> Option<(String, &'static str)> {
    let message = message.trim();
    RULES.iter().zip(compiled()).find_map(|(rule, re)| {
        let captures = re.captures(message)?;
        (rule.fact)(&captures).map(|fact| (fact, rule.category))
    })
}

/// Saves the first fact `message` states, if any.
pub fn remember_from(message: &str) {
    if let Some((fact, category)) = extract(message) {
        if crate::memory::add_fact(&fact, category).0 {
            log::info!("remembered: {fact}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_are_found_and_filler_is_not() {
        assert_eq!(extract("Hi, my name is Aki!"), Some(("User's name is Aki".into(), "personal")));
        assert_eq!(extract("I'm Tired today"), None);
        assert_eq!(extract("my favourite game is Hades"), Some(("User's favorite game is Hades".into(), "favorites")));
        assert_eq!(extract("I love you"), None);
        assert_eq!(extract("i main Jett in Valorant").map(|f| f.1), Some("gaming"));
        assert_eq!(extract("what time is it"), None);
    }
}
