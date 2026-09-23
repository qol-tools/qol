use std::sync::OnceLock;

use regex::Regex;

const PI_CLAIM_SECTIONS: [&str; 4] = [
    "key decisions",
    "constraints & preferences",
    "critical context",
    "done",
];

const CLAUDE_CLAIM_SECTIONS: [&str; 3] = [
    "key technical concepts",
    "errors and fixes",
    "problem solving",
];

const CLAUDE_TEMPLATE_SECTIONS: [&str; 9] = [
    "primary request and intent",
    "key technical concepts",
    "files and code sections",
    "errors and fixes",
    "problem solving",
    "all user messages",
    "pending tasks",
    "current work",
    "optional next step",
];

const MIN_ITEM_CHARS: usize = 40;
const MAX_ITEM_CHARS: usize = 600;

pub fn claim_lines(text: &str) -> Vec<String> {
    let mut raw = pi_items(text);
    raw.extend(claude_items(text));
    raw.sort_by_key(|item| item.offset);
    raw.into_iter()
        .map(|item| finalize(&item.text))
        .filter_map(|text| {
            let truncated = truncate_chars(&text, MAX_ITEM_CHARS);
            (!truncated.ends_with(':') && truncated.chars().count() >= MIN_ITEM_CHARS)
                .then_some(truncated)
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Line<'a> {
    offset: usize,
    text: &'a str,
}

struct RawItem {
    offset: usize,
    text: String,
}

fn pi_items(text: &str) -> Vec<RawItem> {
    collect_items(&lines(text), &PI_CLAIM_SECTIONS, pi_heading_name)
}

fn claude_items(text: &str) -> Vec<RawItem> {
    collect_items(&lines(text), &CLAUDE_CLAIM_SECTIONS, claude_heading_name)
}

fn lines(text: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for line in text.split('\n') {
        out.push(Line { offset, text: line });
        offset += line.len() + 1;
    }
    out
}

fn collect_items(
    all_lines: &[Line<'_>],
    claim_sections: &[&str],
    heading_name: fn(&str) -> Option<String>,
) -> Vec<RawItem> {
    let mut items: Vec<RawItem> = Vec::new();
    let mut body: Vec<Line<'_>> = Vec::new();
    let mut claim = false;
    let mut fenced = false;
    for line in all_lines {
        if line.text.trim().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some(name) = heading_name(line.text) {
            if claim {
                items.extend(section_items(&body));
            }
            body.clear();
            claim = claim_sections.contains(&name.as_str());
            continue;
        }
        if claim {
            body.push(*line);
        }
    }
    if claim {
        items.extend(section_items(&body));
    }
    items
}

fn section_items(body: &[Line<'_>]) -> Vec<RawItem> {
    let mut items: Vec<RawItem> = Vec::new();
    let mut current: Option<RawItem> = None;
    let mut prose: Vec<&str> = Vec::new();
    let mut prose_offset = 0usize;
    for line in body {
        let trimmed = line.text.trim();
        if trimmed.is_empty() {
            continue;
        }
        if prose.is_empty() {
            prose_offset = line.offset;
        }
        prose.push(trimmed);
        if let Some(rest) = item_marker_rest(trimmed) {
            if let Some(item) = current.take() {
                items.push(item);
            }
            current = Some(raw_item(line.offset, rest.to_string()));
            continue;
        }
        if let Some(item) = current.as_mut() {
            item.text.push(' ');
            item.text.push_str(trimmed);
        }
    }
    if let Some(item) = current.take() {
        items.push(item);
    }
    if items.is_empty() && !prose.is_empty() {
        let text = prose.join(" ");
        items.push(raw_item(prose_offset, text));
    }
    items
}

fn raw_item(offset: usize, text: String) -> RawItem {
    RawItem { offset, text }
}

fn item_marker_rest(trimmed: &str) -> Option<&str> {
    let first = trimmed.chars().next()?;
    match first {
        '-' | '*' | '\u{2022}' => Some(&trimmed[first.len_utf8()..]),
        '0'..='9' => {
            let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
            let after_digits = &trimmed[digits..];
            let punct = after_digits.chars().next()?;
            if punct != '.' && punct != ')' {
                return None;
            }
            let after_punct = &after_digits[punct.len_utf8()..];
            after_punct.starts_with(' ').then_some(after_punct)
        }
        _ => None,
    }
}

fn pi_heading_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if hashes == 0 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(section_name(trimmed))
}

fn claude_heading_name(line: &str) -> Option<String> {
    let caps = claude_heading_re().captures(line)?;
    let name = section_name(caps.get(1).map(|m| m.as_str()).unwrap_or_default());
    CLAUDE_TEMPLATE_SECTIONS
        .contains(&name.as_str())
        .then_some(name)
}

fn section_name(trimmed: &str) -> String {
    let stripped = trimmed.trim_matches(|ch| matches!(ch, '#' | '*') || ch.is_whitespace());
    let without_colon = stripped.strip_suffix(':').unwrap_or(stripped);
    without_colon.trim().to_lowercase()
}

fn claude_heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*\d+\.\s+(.+?):?\s*$").expect("claude heading regex"))
}

fn finalize(raw: &str) -> String {
    let without_bold = raw.replace("**", "");
    let without_checkbox = strip_checkbox(without_bold.trim_start());
    collapse_ws(without_checkbox.trim_matches(|ch| ch == '*' || ch == '_'))
}

fn strip_checkbox(text: &str) -> &str {
    for marker in ["[x] ", "[X] ", "[ ] "] {
        if let Some(rest) = text.strip_prefix(marker) {
            return rest;
        }
    }
    text
}

fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PI_SUMMARY: &str = concat!(
        "## Setup\n",
        "\n",
        "Preamble paragraph that mentions - dashes but stays outside the claim sections.\n",
        "\n",
        "## Key Decisions\n",
        "- Ship the deterministic distill before the daemon restart\n",
        "  and mention the rollout followup in the same line\n",
        "- Keep the lock file short lived and release it on drop\n",
        "\n",
        "```\n",
        "- not an item inside a fence at all\n",
        "```\n",
        "- Trailing decision after the fenced block ends here\n",
        "\n",
        "## Done\n",
        "- Short one\n",
        "\n",
        "## Critical Context\n",
        "The context paragraph fallback stands alone without any item markers.\n",
    );

    const CLAUDE_SUMMARY: &str = concat!(
        "This session is being continued from a previous conversation.\n",
        "\n",
        "1. Key Technical Concepts\n",
        "   - The daemon holds a warm cache across **every** request path\n",
        "2. Errors and Fixes\n",
        "   - Fixed the watcher race by draining batches first\n",
        "     before the seal could flip underneath it\n",
        "   3. Also hardened the seal fallback path for good measure\n",
        "3. Pending Tasks\n",
        "   - Nothing left to finish tonight at all\n",
        "4. Problem Solving\n",
        "Solved the flaky lock test with a stale takeover retry.\n",
    );

    #[test]
    fn claim_lines_parses_pi_shaped_summaries() {
        let lines = claim_lines(PI_SUMMARY);
        assert_eq!(
            lines,
            vec![
                "Ship the deterministic distill before the daemon restart and mention the rollout followup in the same line",
                "Keep the lock file short lived and release it on drop",
                "Trailing decision after the fenced block ends here",
                "The context paragraph fallback stands alone without any item markers.",
            ]
        );
    }

    #[test]
    fn claim_lines_parses_claude_shaped_summaries() {
        let lines = claim_lines(CLAUDE_SUMMARY);
        assert_eq!(
            lines,
            vec![
                "The daemon holds a warm cache across every request path",
                "Fixed the watcher race by draining batches first before the seal could flip underneath it",
                "Also hardened the seal fallback path for good measure",
                "Solved the flaky lock test with a stale takeover retry.",
            ]
        );
    }

    #[test]
    fn claim_lines_drops_short_items_and_truncates_long_ones() {
        let long = "x".repeat(700);
        let text = format!("## Key Decisions\n- {long}\n- too short\n");
        let lines = claim_lines(&text);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].chars().count(), MAX_ITEM_CHARS);
    }

    #[test]
    fn claim_lines_applies_the_live_run_filters() {
        let text = concat!(
            "## Key Decisions\n",
            "- [x] Ship the checkbox marker handling in the distill section parser\n",
            "- Heading fragment that leaks from the section header line:\n",
            "- exactly thirty characters long\n",
        );
        let lines = claim_lines(text);
        assert_eq!(
            lines,
            vec!["Ship the checkbox marker handling in the distill section parser"]
        );
    }
}
