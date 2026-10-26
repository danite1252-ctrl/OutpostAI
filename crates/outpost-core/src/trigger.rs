//! Trigger words: the made-up word that calls a trained character up in a prompt.
//!
//! A good trigger is a word the model has never seen. A real word or a first name
//! ("kat", "danni", "rexdog") already means something to it, so the training fights
//! that meaning and the character comes out weaker or mixed up. [`check`] says
//! whether a word is safe and suggests one that is (`danni` → `zd4nni`).

use std::collections::HashSet;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const COMMON: &str = include_str!("words/common.txt");
const NAMES: &str = include_str!("words/names.txt");

fn words(text: &str) -> HashSet<&str> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .flat_map(str::split_whitespace)
        .filter(|w| w.len() >= 3)
        .collect()
}

fn common() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| words(COMMON))
}

fn names() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| words(NAMES))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TriggerCheck {
    /// The word as it will be used (lower case).
    pub word: String,
    pub ok: bool,
    /// Why it isn't safe, in plain words.
    pub problem: Option<String>,
    /// A safe word close to it, when this one isn't.
    pub suggestion: Option<String>,
}

/// Letters only, digits read back as the letters they stand for (`d4nni` → `danni`).
fn unleet(word: &str) -> String {
    word.chars()
        .map(|c| match c {
            '4' => 'a',
            '3' => 'e',
            '1' => 'i',
            '0' => 'o',
            '5' => 's',
            '7' => 't',
            c => c,
        })
        .filter(char::is_ascii_alphabetic)
        .collect()
}

/// What's wrong with a lower-case word, if anything.
fn problem(word: &str) -> Option<String> {
    if word.len() < 4 {
        return Some("It's very short; use at least 4 letters and numbers.".into());
    }
    if word.len() > 24 {
        return Some("It's long; keep it under 25 characters.".into());
    }
    if !word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return Some("Use only letters and numbers, no spaces.".into());
    }
    if !word.chars().any(|c| c.is_ascii_lowercase()) {
        return Some("Include some letters.".into());
    }
    let (common, names) = (common(), names());
    let plain = unleet(word);
    if names.contains(plain.as_str()) {
        return Some(format!("\"{plain}\" is a name the model already knows."));
    }
    if common.contains(plain.as_str()) {
        return Some(format!("\"{plain}\" is a word the model already knows."));
    }
    // Each run of letters between digits: a real word in it, at either end, shows through.
    for run in word.split(|c: char| c.is_ascii_digit()).filter(|r| r.len() >= 3) {
        if names.contains(run) {
            return Some(format!("It contains the name \"{run}\"."));
        }
        for len in (3..=run.len()).rev() {
            for part in [&run[..len], &run[run.len() - len..]] {
                if common.contains(part) {
                    return Some(format!("It contains the word \"{part}\"."));
                }
            }
        }
    }
    None
}

/// Swap the first plain vowel (after the first letter) for a look-alike digit.
fn leet_next(word: &str) -> Option<String> {
    let mut chars: Vec<char> = word.chars().collect();
    let i = chars
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, c)| matches!(c, 'a' | 'e' | 'i' | 'o'))?
        .0;
    chars[i] = match chars[i] {
        'a' => '4',
        'e' => '3',
        'i' => '1',
        _ => '0',
    };
    Some(chars.into_iter().collect())
}

/// A safe word close to `word`: digits for a vowel, a `z` in front, then more digits
/// until nothing shows through.
fn suggest(word: &str) -> Option<String> {
    let mut w: String = word.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if w.is_empty() {
        return None;
    }
    if !w.chars().any(|c| c.is_ascii_digit()) {
        if let Some(next) = leet_next(&w) {
            w = next;
        }
    }
    if !w.starts_with('z') {
        w.insert(0, 'z');
    }
    while w.len() < 4 {
        w.push('x');
    }
    for _ in 0..8 {
        if problem(&w).is_none() {
            return Some(w);
        }
        w = leet_next(&w).unwrap_or_else(|| format!("{w}x"));
    }
    None
}

/// Is `word` a safe trigger? If not, why, and what to use instead.
pub fn check(word: &str) -> TriggerCheck {
    let word = word.trim().to_lowercase();
    match problem(&word) {
        None => TriggerCheck {
            word,
            ok: true,
            problem: None,
            suggestion: None,
        },
        Some(p) => TriggerCheck {
            suggestion: suggest(&word),
            word,
            ok: false,
            problem: Some(p),
        },
    }
}

/// A first suggestion from a character's name, for a new dataset.
pub fn from_name(name: &str) -> String {
    let base: String = name
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(12)
        .collect();
    let base = if base.is_empty() { "char".to_string() } else { base };
    if problem(&base).is_none() {
        return base;
    }
    suggest(&base).unwrap_or_else(|| format!("z{base}x"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_words_and_names_are_flagged() {
        assert!(!check("danni").ok);
        assert!(!check("kat").ok);
        assert!(!check("Kate").ok);
        assert!(!check("rexdog").ok, "ends with dog");
        assert!(!check("d4nni").ok, "reads as danni");
        assert!(!check("my dog").ok);
        assert!(check("zd4nni").ok);
        assert!(check("zk4t9").ok);
    }

    #[test]
    fn suggestions_pass_their_own_check() {
        assert_eq!(check("danni").suggestion.as_deref(), Some("zd4nni"));
        assert_eq!(check("d4nni").suggestion.as_deref(), Some("zd4nni"));
        assert_eq!(check("rexdog").suggestion.as_deref(), Some("zr3xd0g"));
        for w in ["kat", "kate", "sam", "bob", "princess", "redcat", "abc", "anna", "lily"] {
            let c = check(w);
            assert!(!c.ok, "{w}");
            let s = c.suggestion.unwrap_or_else(|| panic!("no suggestion for {w}"));
            assert!(check(&s).ok, "{w} -> {s} should pass");
        }
    }

    #[test]
    fn names_become_triggers() {
        assert!(check(&from_name("Kat")).ok);
        assert!(check(&from_name("Danni")).ok);
        assert!(check(&from_name("  ")).ok);
        assert_eq!(from_name("Danni"), "zd4nni");
    }
}
