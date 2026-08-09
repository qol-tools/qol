const STATUS_TAIL: usize = 8;
const CLAUDE_STATUS_TAIL: usize = 16;
const KIMI_STATUS_TAIL: usize = 30;
const CHOICE_HINTS: [&str; 6] = [
    "enter to",
    "to confirm",
    "to select",
    "to navigate",
    "to submit",
    "space to toggle",
];

pub(super) fn has_interrupt_hint(text: &str) -> bool {
    tail(text, STATUS_TAIL)
        .iter()
        .any(|line| line.contains("esc to interrupt"))
}

pub(super) fn claude_working(text: &str) -> bool {
    tail(text, CLAUDE_STATUS_TAIL).iter().any(|line| {
        let trimmed = line.trim();
        trimmed.contains("esc to interrupt") || is_status_spinner(trimmed)
    })
}

fn is_status_spinner(trimmed: &str) -> bool {
    let Some((glyph, rest)) = trimmed.split_once(' ') else {
        return false;
    };
    let single = glyph.chars().count() == 1;
    single
        && !glyph.chars().all(char::is_alphanumeric)
        && !starts_with_glyph(glyph, 0x2800..=0x28FF)
        && rest.contains("\u{2026} (")
        && trimmed.ends_with(')')
}

pub(super) fn has_braille_spinner(text: &str) -> bool {
    tail(text, STATUS_TAIL).iter().any(|line| {
        let trimmed = line.trim_start();
        starts_with_glyph(trimmed, 0x2800..=0x28FF)
            && (trimmed.contains("...") || trimmed.contains('\u{2026}'))
    })
}

pub(super) fn has_choice_arrows(text: &str) -> bool {
    tail(text, STATUS_TAIL).iter().any(|line| {
        let trimmed = line.trim();
        trimmed.contains('\u{2191}') && trimmed.contains("navigate") && trimmed.contains("select")
    })
}

pub(super) fn has_numbered_choice(text: &str) -> bool {
    let tail = tail(text, STATUS_TAIL);
    tail.iter().any(|line| numbered_option(line.trim()))
        && tail.iter().any(|line| is_choice_affordance(line))
}

pub(super) fn has_done_marker(text: &str) -> bool {
    tail(text, CLAUDE_STATUS_TAIL).iter().any(|line| {
        let trimmed = line.trim();
        starts_with_glyph(trimmed, 0x2733..=0x273F)
            && trimmed.contains(" for ")
            && ends_with_duration(trimmed)
    })
}

pub(super) fn kimi_working(text: &str) -> bool {
    let tail = kimi_status_tail(text);
    tail.iter().any(|line| is_kimi_spinner(line.trim_start()))
        || tail.windows(2).any(|window| {
            is_bare_moon(window[0].trim_start()) && is_editor_box_top(window[1].trim_start())
        })
}

pub(super) fn kimi_questionnaire(text: &str) -> bool {
    let tail = kimi_status_tail(text);
    let numbered = tail.iter().any(|line| numbered_option(line.trim()));
    let footer = tail.iter().any(|line| {
        let lower = line.to_lowercase();
        lower.contains("tab switch")
            && lower.contains("esc cancel")
            && (lower.contains("\u{21B5} save") || lower.contains("\u{21B5} choose"))
    });
    numbered && footer
}

pub(super) fn contains_any(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| text.contains(marker))
}

fn tail(text: &str, window: usize) -> Vec<&str> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(window)
        .collect()
}

fn kimi_status_tail(text: &str) -> Vec<&str> {
    tail(text, KIMI_STATUS_TAIL).into_iter().rev().collect()
}

fn is_kimi_spinner(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if is_moon_phase(first) {
        chars
            .as_str()
            .strip_prefix('\u{FE0F}')
            .unwrap_or(chars.as_str())
            .trim_start()
            .starts_with('\u{00B7}')
    } else if matches!(first as u32, 0x2800..=0x28FF) {
        text.contains("...") || text.contains('\u{2026}')
    } else {
        false
    }
}

fn is_bare_moon(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    is_moon_phase(first)
        && chars
            .as_str()
            .strip_prefix('\u{FE0F}')
            .unwrap_or(chars.as_str())
            .trim()
            .is_empty()
}

fn is_moon_phase(character: char) -> bool {
    matches!(character as u32, 0x1F311..=0x1F318)
}

fn is_editor_box_top(text: &str) -> bool {
    matches!(text.chars().next(), Some('\u{256D}'))
}

fn starts_with_glyph(text: &str, range: std::ops::RangeInclusive<u32>) -> bool {
    text.chars()
        .next()
        .is_some_and(|character| range.contains(&(character as u32)))
}

fn ends_with_duration(text: &str) -> bool {
    let tail = text.rsplit(" for ").next().unwrap_or("");
    let tokens = tail.split_whitespace().collect::<Vec<_>>();
    !tokens.is_empty() && tokens.iter().all(|token| is_duration_token(token))
}

fn is_duration_token(token: &str) -> bool {
    let Some(unit) = token.chars().last() else {
        return false;
    };
    if !matches!(unit, 'h' | 'm' | 's') {
        return false;
    }
    let number = &token[..token.len() - unit.len_utf8()];
    !number.is_empty() && number.chars().all(|character| character.is_ascii_digit())
}

fn numbered_option(text: &str) -> bool {
    let text = text
        .strip_prefix('\u{2192}')
        .map(str::trim_start)
        .unwrap_or(text);
    if bracketed_numbered_option(text) {
        return true;
    }
    let digits = text
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digits == 0 {
        return false;
    }
    let mut rest = text[digits..].chars();
    if !matches!(rest.next(), Some('.') | Some(')')) {
        return false;
    }
    matches!(rest.next(), None | Some(' ') | Some('\t'))
}

fn bracketed_numbered_option(text: &str) -> bool {
    let Some(text) = text.strip_prefix('[') else {
        return false;
    };
    let digits = text
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digits == 0 {
        return false;
    }
    let mut rest = text[digits..].chars();
    if rest.next() != Some(']') {
        return false;
    }
    matches!(rest.next(), None | Some(' ') | Some('\t'))
}

fn is_choice_affordance(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.starts_with("[x]") || trimmed.starts_with("[ ]") {
        return true;
    }
    let lower = trimmed.to_lowercase();
    lower.contains("[y/n]")
        || lower.contains("(y/n)")
        || lower.contains("\u{2191}\u{2193} select")
        || lower.contains("\u{21B5} choose")
        || CHOICE_HINTS.iter().any(|hint| lower.contains(hint))
}

#[cfg(test)]
mod tests {
    use super::{
        claude_working, contains_any, has_braille_spinner, has_choice_arrows, has_done_marker,
        has_interrupt_hint, has_numbered_choice, kimi_questionnaire, kimi_working,
    };

    fn screen(tail: &[&str]) -> String {
        let mut lines = vec!["$ echo done", "done", ""];
        lines.extend_from_slice(tail);
        lines.join("\n")
    }

    #[test]
    fn interrupt_hint_is_detected_in_the_recent_tail() {
        let cases = [
            ("  esc to interrupt ", true),
            ("esc to interrupt", true),
            ("esc to cancel", false),
            ("press esc to interrupt now", true),
        ];
        for (line, expected) in cases {
            let text = screen(&[line]);
            assert_eq!(has_interrupt_hint(&text), expected, "line: {line}");
        }
    }

    #[test]
    fn interrupt_hint_ignores_stale_tail_lines() {
        let mut text = String::new();
        for _ in 0..20 {
            text.push_str("esc to interrupt\n");
        }
        for _ in 0..9 {
            text.push_str("filler\n");
        }
        assert!(!has_interrupt_hint(&text));
    }

    #[test]
    fn claude_work_needs_an_interrupt_hint_or_an_ellipsis_badge() {
        let cases = [
            ("\u{2728} Working \u{2026} (2s)", true),
            ("\u{2728} Working... (2s)", false),
            ("\u{2728} Working \u{2026} 2s", false),
            ("working \u{2026} (2s)", false),
            ("\u{273F} thinking \u{2026} (12s)", true),
            ("\u{2800} \u{2026} (1s)", false),
            (
                "* Pontificating\u{2026} (49s \u{b7} \u{2193} 6.4k tokens)",
                true,
            ),
        ];
        for (line, expected) in cases {
            let text = screen(&[line]);
            assert_eq!(claude_working(&text), expected, "line: {line}");
        }
    }

    #[test]
    fn braille_spinner_requires_an_ellipsis_marker() {
        let cases = [
            ("\u{2800}\u{2801} thinking...", true),
            ("\u{2800}\u{2801} thinking\u{2026}", true),
            ("\u{2800}\u{2801} thinking", false),
            ("thinking...", false),
        ];
        for (line, expected) in cases {
            let text = screen(&[line]);
            assert_eq!(has_braille_spinner(&text), expected, "line: {line}");
        }
    }

    #[test]
    fn choice_arrows_require_all_three_tokens() {
        let cases = [
            ("\u{2191}\u{2193} to navigate, \u{21B5} to select", true),
            ("\u{2191} navigate", false),
            ("select with \u{2191}", false),
            ("navigate select", false),
        ];
        for (line, expected) in cases {
            let text = screen(&[line]);
            assert_eq!(has_choice_arrows(&text), expected, "line: {line}");
        }
    }

    #[test]
    fn numbered_choice_needs_both_an_option_and_an_affordance() {
        let cases = [
            ("1) rewrite the loop\nenter to accept", true),
            ("[2] open file\nspace to toggle", true),
            ("\u{2192} 3) run tests\nto select", true),
            ("1) rewrite the loop", false),
            ("enter to accept", false),
            ("99. next\n(y/n)", true),
            ("1)one\nenter", false),
            ("12) pick one\n\u{2191}\u{2193} select", true),
        ];
        for (lines, expected) in cases {
            let text = screen(&lines.split('\n').collect::<Vec<_>>());
            assert_eq!(has_numbered_choice(&text), expected, "lines: {lines}");
        }
    }

    #[test]
    fn done_marker_requires_a_star_with_duration() {
        let cases = [
            ("\u{2734} Fixed the queue for 12s", true),
            ("\u{2734} Fixed for 1m 2s", true),
            ("\u{2734} Fixed the queue", false),
            ("\u{2734} Fixed for 12", false),
            ("Fixed the queue for 12s", false),
            ("\u{2734} Fixed for 2h 3m", true),
        ];
        for (line, expected) in cases {
            let text = screen(&[line]);
            assert_eq!(has_done_marker(&text), expected, "line: {line}");
        }
    }

    #[test]
    fn kimi_working_spans_spinners_and_editor_boxes() {
        let cases = [
            (vec!["\u{1F311}\u{FE0F} \u{00B7} building"], true),
            (vec!["\u{1F313} \u{00B7} thinking"], true),
            (vec!["\u{2800} \u{00B7} running..."], true),
            (vec!["\u{1F311}\u{FE0F} building"], false),
            (vec!["\u{1F311}\u{FE0F} \u{00B7} idle"], true),
            (vec!["\u{1F311}\u{FE0F} idle"], false),
            (vec!["\u{1F311}", "\u{256D} editor"], true),
            (vec!["\u{1F311}\u{FE0F} work", "\u{256D} editor"], false),
        ];
        for (lines, expected) in cases {
            let text = screen(&lines);
            assert_eq!(kimi_working(&text), expected, "lines: {lines:?}");
        }
    }

    #[test]
    fn kimi_questionnaire_requires_numbered_options_and_the_footer() {
        let cases = [
            (
                vec![
                    "1) quick",
                    "2) thorough",
                    "tab switch to change, esc cancel, \u{21B5} save",
                ],
                true,
            ),
            (
                vec![
                    "1) quick",
                    "2) thorough",
                    "tab switch, esc cancel, \u{21B5} choose",
                ],
                true,
            ),
            (
                vec!["1) quick", "2) thorough", "tab switch, esc cancel"],
                false,
            ),
            (vec!["1) quick", "2) thorough", "\u{21B5} save"], false),
            (
                vec!["quick", "thorough", "tab switch, esc cancel, \u{21B5} save"],
                false,
            ),
        ];
        for (lines, expected) in cases {
            let text = screen(&lines);
            assert_eq!(kimi_questionnaire(&text), expected, "lines: {lines:?}");
        }
    }

    #[test]
    fn contains_any_matches_any_marker() {
        let text = "OpenAI Codex (v0.40) started";
        assert!(contains_any(
            text,
            &["OpenAI Codex (v", "Tip: Try the Codex App"]
        ));
        assert!(!contains_any(text, &["Tip: Try the Codex App"]));
        assert!(contains_any(
            "to show full startup help",
            &["to show full startup help"]
        ));
    }

    #[test]
    fn markers_survive_arbitrary_unicode_text() {
        let alphabet =
            "\u{00B7}\u{2026}\u{2191}\u{2193}\u{21B5}\u{256D}\u{2728}\u{2734}\u{2800}\u{1F311}";
        for length in 0..24 {
            for _ in 0..8 {
                let mut text = String::new();
                let characters = alphabet.chars().collect::<Vec<_>>();
                let mut seed = (length as u64) * 7919 + 104729;
                for _ in 0..length {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    let index = (seed >> 33) as usize % characters.len();
                    text.push(characters[index]);
                }
                text.push('\n');
                let _ = (
                    has_interrupt_hint(&text),
                    claude_working(&text),
                    has_braille_spinner(&text),
                    has_choice_arrows(&text),
                    has_numbered_choice(&text),
                    has_done_marker(&text),
                    kimi_working(&text),
                    kimi_questionnaire(&text),
                );
            }
        }
    }
}
