use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub fn screen_hash(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

const KEY_HINTS: [&str; 6] = [
    "enter to",
    "to confirm",
    "to select",
    "to navigate",
    "to submit",
    "space to toggle",
];

pub fn has_prompt_markers(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.iter().rev().take(8).any(|line| is_selection(line))
}

pub fn claude_working(text: &str) -> bool {
    text.lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(12)
        .any(|l| {
            let t = l.trim();
            t.contains("esc to interrupt") || (t.contains("\u{2026} (") && t.ends_with(')'))
        })
}

pub fn claude_done(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim();
        starts_with_star(t) && t.contains(" for ") && ends_with_duration(t)
    })
}

pub fn codex_working(text: &str) -> bool {
    text.lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(8)
        .any(|l| l.contains("esc to interrupt"))
}

pub fn codex_banner(text: &str) -> bool {
    text.contains("OpenAI Codex (v") || text.contains("Tip: Try the Codex App")
}

fn starts_with_star(t: &str) -> bool {
    matches!(t.chars().next(), Some(c) if (0x2733..=0x273F).contains(&(c as u32)))
}

fn ends_with_duration(t: &str) -> bool {
    let tail = t.rsplit(" for ").next().unwrap_or("");
    let toks: Vec<&str> = tail.split_whitespace().collect();
    !toks.is_empty() && toks.iter().all(|tok| is_duration_token(tok))
}

fn is_duration_token(tok: &str) -> bool {
    let Some(unit) = tok.chars().last() else {
        return false;
    };
    if !matches!(unit, 'h' | 'm' | 's') {
        return false;
    }
    let num = &tok[..tok.len() - unit.len_utf8()];
    !num.is_empty() && num.chars().all(|c| c.is_ascii_digit())
}

pub fn claude_awaiting_choice(text: &str) -> bool {
    text.lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(12)
        .any(|l| {
            let mut chars = l.trim_start().chars();
            matches!(chars.next(), Some('\u{276F}') | Some('\u{203A}'))
                && numbered_option(chars.as_str().trim_start())
        })
}

pub fn has_input_request(text: &str) -> bool {
    match text.lines().rfind(|l| !l.trim().is_empty()) {
        Some(line) => {
            let t = line.trim();
            t.ends_with(':') || t.ends_with('?')
        }
        None => false,
    }
}

fn is_selection(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    if t.starts_with("[x]") || t.starts_with("[ ]") {
        return true;
    }
    if numbered_option(t) {
        return true;
    }
    let lower = t.to_lowercase();
    if lower.contains("[y/n]") || lower.contains("(y/n)") {
        return true;
    }
    KEY_HINTS.iter().any(|h| lower.contains(h))
}

fn numbered_option(t: &str) -> bool {
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return false;
    }
    let mut rest = t[digits..].chars();
    if !matches!(rest.next(), Some('.') | Some(')')) {
        return false;
    }
    matches!(rest.next(), None | Some(' ') | Some('\t'))
}
