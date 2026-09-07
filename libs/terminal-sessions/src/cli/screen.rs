const STATUS_TAIL: usize = 8;
const PI_SPINNER_TAIL: usize = 12;
const CLAUDE_STATUS_TAIL: usize = 16;
const KIMI_STATUS_TAIL: usize = 30;
const FOOTER_BELOW_MIN: usize = 1;
const FOOTER_BELOW_MAX: usize = 6;
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
    tail(text, PI_SPINNER_TAIL).iter().any(|line| {
        let trimmed = line.trim_start();
        starts_with_glyph(trimmed, 0x2800..=0x28FF)
            && (trimmed.contains("...") || trimmed.contains('\u{2026}'))
    })
}

pub(super) fn pi_working(text: &str) -> bool {
    if has_braille_spinner(text) {
        return true;
    }
    let lines = text.lines().collect::<Vec<_>>();
    let Some(bottom) = lines.iter().rposition(|line| is_rule_line(line)) else {
        return false;
    };
    let Some(border) = lines[..bottom]
        .iter()
        .rev()
        .find(|line| line.trim_start().starts_with('─'))
    else {
        return false;
    };
    let status = border.trim().trim_matches('─').trim();
    let Some((indicator, message)) = status.split_once(' ') else {
        return false;
    };
    !indicator.is_empty()
        && indicator
            .chars()
            .all(|character| !character.is_alphanumeric())
        && (message == "Working"
            || message.starts_with("Working ")
            || message.starts_with("Working…")
            || message.starts_with("Working..."))
}

pub(super) fn has_choice_hint(text: &str) -> bool {
    let region = chrome_region(text).unwrap_or_else(|| tail(text, STATUS_TAIL));
    region.iter().any(|line| {
        let lower = line.to_lowercase();
        lower.contains('\u{2191}')
            && lower.contains("select")
            && ["choose", "confirm", "save", "navigate", "toggle"]
                .iter()
                .any(|word| lower.contains(word))
    })
}

pub(super) fn pi_live(text: &str) -> bool {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let Some(last_rule) = lines.iter().rposition(|line| is_rule_line(line)) else {
        return false;
    };
    let below = lines.len() - 1 - last_rule;
    (FOOTER_BELOW_MIN..=FOOTER_BELOW_MAX).contains(&below)
}

pub(super) fn kimi_live(text: &str) -> bool {
    let Some(last) = text.lines().rfind(|line| !line.trim().is_empty()) else {
        return false;
    };
    let last = last.trim();
    is_rule_line(last)
        || (last.contains("context: ") && (last.ends_with('%') || last.ends_with(')')))
        || (kimi_hint_line(last) && kimi_dialog_corpus(text))
}

fn is_rule_line(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.chars().all(|character| character == '\u{2500}')
}

fn chrome_region(text: &str) -> Option<Vec<&str>> {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let rules: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| is_rule_line(line))
        .map(|(index, _)| index)
        .collect();
    let last_rule = *rules.last()?;
    let start = match rules.len() {
        0 => return None,
        1 | 2 => 0,
        3 => rules[0] + 1,
        _ => rules[rules.len() - 4] + 1,
    };
    Some(lines[start..last_rule].to_vec())
}

pub(super) fn has_picker_cluster(text: &str) -> bool {
    let Some(region) = chrome_region(text) else {
        return false;
    };
    let mut search = None;
    let mut selected = None;
    let mut counter = None;
    for (index, line) in region.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed == ">" || trimmed.starts_with("> ") {
            search.get_or_insert(index);
        } else if line.trim_start().starts_with('\u{2192}') {
            selected.get_or_insert(index);
        } else if counter_pair(trimmed).is_some() {
            counter.get_or_insert(index);
        }
    }
    let (Some(search), Some(selected), Some(counter)) = (search, selected, counter) else {
        return false;
    };
    let after = &region[counter + 1..];
    let info_lines = after.iter().take_while(|line| !is_rule_line(line.trim()));
    let adjacent = info_lines.clone().count() <= 3
        && info_lines
            .clone()
            .all(|line| !is_picker_marker(line.trim()))
        && after
            .iter()
            .filter(|line| is_rule_line(line.trim()))
            .count()
            <= 2
        && after
            .iter()
            .rev()
            .take_while(|line| !is_rule_line(line.trim()))
            .count()
            <= 2;
    search < selected && selected < counter && adjacent
}

fn is_picker_marker(trimmed: &str) -> bool {
    counter_pair(trimmed).is_some()
        || trimmed.starts_with('\u{2192}')
        || trimmed == ">"
        || trimmed.starts_with("> ")
}

fn counter_pair(trimmed: &str) -> Option<(&str, &str)> {
    let inner = trimmed.strip_prefix('(')?;
    let (count, rest) = inner.split_once('/')?;
    let total = rest.strip_suffix(')')?;
    let digits = |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(count) || !digits(total) {
        return None;
    }
    Some((count, total))
}

fn kimi_hint_line(line: &str) -> bool {
    line.contains("esc cancel")
        || line.contains("\u{21B5} confirm")
        || line.contains("\u{21B5} choose")
        || line.contains("\u{21B5} save")
}

fn kimi_dialog_corpus(text: &str) -> bool {
    tail(text, KIMI_STATUS_TAIL).iter().any(|line| {
        let trimmed = line.trim();
        numbered_option(trimmed)
            || trimmed.contains('\u{25B6}')
            || trimmed.contains('\u{2713}')
            || trimmed.contains('\u{25CB}')
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

pub fn provider_error_line(text: &str) -> Option<&str> {
    let line = transcript_region(text)
        .into_iter()
        .rev()
        .find(|line| !is_progress_line(line))?;
    let trimmed = line.trim();
    let detail = trimmed.strip_prefix("Error: ")?;
    (!detail.trim().is_empty()).then_some(trimmed)
}

fn transcript_region(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let footer = lines
        .iter()
        .rev()
        .take_while(|line| !is_rule_line(line))
        .count();
    let rules = lines.len() - footer;
    let start = lines[..rules]
        .iter()
        .rposition(|line| !is_rule_line(line))
        .map(|index| index + 1)
        .unwrap_or(0);
    lines[..start].to_vec()
}

fn is_progress_line(line: &str) -> bool {
    let bare = activity_signature(line);
    bare == "Thinking..." || bare == "Working..." || bare.starts_with("Took ")
}

pub fn editor_draft(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let last = lines.iter().rposition(|line| is_rule_line(line))?;
    let first = lines[..last].iter().rposition(|line| is_rule_line(line))?;
    let draft = lines[first + 1..last]
        .iter()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    (!draft.trim().is_empty()).then_some(draft)
}

pub fn activity_signature(text: &str) -> String {
    let mut signature = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.chars() {
        if is_animation_glyph(character) {
            continue;
        }
        if character.is_whitespace() {
            pending_space = !signature.is_empty();
            continue;
        }
        if pending_space {
            signature.push(' ');
            pending_space = false;
        }
        signature.push(character);
    }
    signature
}

fn is_animation_glyph(character: char) -> bool {
    matches!(character as u32, 0x2800..=0x28FF)
        || is_moon_phase(character)
        || character == '\u{FE0F}'
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
        claude_working, contains_any, has_braille_spinner, has_choice_hint, has_done_marker,
        has_interrupt_hint, has_numbered_choice, has_picker_cluster, kimi_live, kimi_questionnaire,
        kimi_working, pi_live,
    };

    fn screen(tail: &[&str]) -> String {
        let mut lines = vec!["$ echo done", "done", ""];
        lines.extend_from_slice(tail);
        lines.join("\n")
    }

    fn pi_screen(tail: &[&str]) -> String {
        let mut lines = vec!["conversation output", ""];
        lines.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        lines.extend_from_slice(tail);
        lines.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        lines.push("  draft line");
        lines.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        lines.push("/work/proj (main)");
        lines.push("$0.400 47.3%/1.0M (auto)");
        lines.join("\n")
    }

    fn kimi_screen(tail: &[&str]) -> String {
        let mut lines = vec!["conversation output", ""];
        lines.extend_from_slice(tail);
        lines.push("yolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]");
        lines.push("context: 17% (41.1k/256k)");
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
    fn choice_hint_matches_every_real_arrow_hint_family() {
        let cases = [
            ("\u{2191}\u{2193} navigate, \u{21B5} to select", true),
            (
                "  \u{2191}\u{2193} select  1-3 / \u{21B5} choose  tab switch  esc cancel",
                true,
            ),
            (
                "  \u{2191}\u{2193} select  1/2 choose  \u{21B5} confirm  tab switch  esc cancel",
                true,
            ),
            (
                "  \u{2191}\u{2193} select \u{00B7} 1/2 choose \u{00B7} \u{21B5} confirm",
                true,
            ),
            ("\u{2191} navigate select", true),
            ("select with \u{2191}", false),
            ("navigate select", false),
            ("\u{2191}\u{2193} scroll, then select a file", false),
        ];
        for (line, expected) in cases {
            let text = screen(&[line]);
            assert_eq!(has_choice_hint(&text), expected, "line: {line}");
        }
    }

    #[test]
    fn choice_hint_survives_a_tall_transcript_above_the_dialog() {
        let mut lines =
            vec!["\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}".to_string()];
        for index in 0..40 {
            lines.push(format!("older transcript line {index}"));
        }
        lines.push(
            "  \u{2191}\u{2193} select  1/2 choose  \u{21B5} confirm  tab switch  esc cancel"
                .to_string(),
        );
        lines.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}".to_string());
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let text = kimi_screen(&refs);
        assert!(has_choice_hint(&text));
    }

    #[test]
    fn picker_cluster_requires_selected_row_counter_and_search() {
        let picker = vec![
            ">",
            "",
            "\u{2192} deepseek-v4-flash [deepseek] \u{2713}",
            "  deepseek-v4-pro [deepseek]",
            "  k3 [kimi-coding]",
            "  (1/13)",
            "",
        ];
        let text = pi_screen(&picker);
        assert!(has_picker_cluster(&text));

        let mut missing_search = picker.clone();
        missing_search[0] = "";
        assert!(!has_picker_cluster(&pi_screen(&missing_search)));

        let mut missing_counter = picker.clone();
        missing_counter[5] = "  2 sessions";
        assert!(!has_picker_cluster(&pi_screen(&missing_counter)));

        let mut missing_arrow = picker.clone();
        missing_arrow[2] = "  deepseek-v4-flash [deepseek]";
        assert!(!has_picker_cluster(&pi_screen(&missing_arrow)));

        let decoy = vec![
            "> blockquote in a transcript",
            "\u{2192} item from tool output",
            "  (1/13)",
        ];
        let mut transcript = vec!["conversation output", ""];
        transcript.extend_from_slice(&decoy);
        transcript.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        transcript.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        transcript.push("  draft line");
        transcript.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        transcript.push("/work/proj (main)");
        transcript.push("$0.400 47.3%/1.0M (auto)");
        assert!(
            !has_picker_cluster(&transcript.join("\n")),
            "a transcript triple with a parseable counter above the chat border is not a picker"
        );

        let mut in_region = vec!["conversation output", ""];
        in_region.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        in_region.extend_from_slice(&decoy);
        in_region.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        in_region.push("  draft line");
        in_region.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        in_region.push("/work/proj (main)");
        in_region.push("$0.400 47.3%/1.0M (auto)");
        assert!(
            has_picker_cluster(&in_region.join("\n")),
            "a status-area triple is geometrically identical to a picker"
        );

        let mut unordered = picker.clone();
        unordered[2] = "  (1/13)";
        unordered[5] = "\u{2192} deepseek-v4-flash [deepseek] \u{2713}";
        assert!(
            !has_picker_cluster(&pi_screen(&unordered)),
            "the counter must sit below the selected row"
        );
    }

    #[test]
    fn picker_cluster_tolerates_info_lines_below_the_counter() {
        let picker = vec![
            ">",
            "\u{2192} deepseek-v4-flash [deepseek] \u{2713}",
            "  k3 [kimi-coding]",
            "  (1/13)",
            "  Model Name: DeepSeek V4 Flash",
            "  Model catalogs refreshed.",
            "",
        ];
        let text = pi_screen(&picker);
        assert!(has_picker_cluster(&text));
    }

    #[test]
    fn picker_cluster_survives_a_rule_below_the_status_area() {
        let mut lines = vec!["conversation output", ""];
        lines.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        lines.push(">");
        lines.push("\u{2192} deepseek-v4-flash [deepseek] \u{2713}");
        lines.push("  k3 [kimi-coding]");
        lines.push("  (1/13)");
        lines.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        lines.push("  draft line");
        lines.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        lines.push("/work/proj (main)");
        lines.push("$0.400 47.3%/1.0M (auto)");
        assert!(
            has_picker_cluster(&lines.join("\n")),
            "a rule below the status area must not push the picker out of the region"
        );
    }

    #[test]
    fn picker_cluster_accepts_a_counter_shaped_draft_while_the_picker_is_open() {
        let picker = vec![
            ">",
            "\u{2192} deepseek-v4-flash [deepseek] \u{2713}",
            "  (1/13)",
        ];
        let mut lines = vec!["conversation output", ""];
        lines.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        lines.extend_from_slice(&picker);
        lines.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        lines.push("  (1/13)");
        lines.push("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        lines.push("/work/proj (main)");
        lines.push("$0.400 47.3%/1.0M (auto)");
        assert!(has_picker_cluster(&lines.join("\n")));
    }

    #[test]
    fn picker_cluster_survives_a_tall_list() {
        let mut picker = vec![">".to_string()];
        picker.push("  deepseek-v4-flash [deepseek]".to_string());
        picker.push("\u{2192} deepseek-v4-flash [deepseek] \u{2713}".to_string());
        for index in 0..33 {
            picker.push(format!("  option number {index}"));
        }
        picker.push("  (1/13)".to_string());
        let refs: Vec<&str> = picker.iter().map(String::as_str).collect();
        assert!(has_picker_cluster(&pi_screen(&refs)));
    }

    #[test]
    fn picker_cluster_is_anchored_to_the_chrome_not_the_tail() {
        let mut lines =
            vec!["\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}".to_string()];
        for index in 0..40 {
            lines.push(format!("older transcript line {index}"));
        }
        lines.push(">".to_string());
        lines.push(String::new());
        lines.push("\u{2192} deepseek-v4-flash [deepseek] \u{2713}".to_string());
        lines.push("  k3 [kimi-coding]".to_string());
        lines.push("  (1/13)".to_string());
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let text = pi_screen(&refs);
        assert!(has_picker_cluster(&text));
    }

    #[test]
    fn panned_frames_without_live_chrome_are_rejected() {
        let mut lines =
            vec!["\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}".to_string()];
        for index in 0..40 {
            lines.push(format!("panned history line {index}"));
        }
        let text = lines.join("\n");
        assert!(!pi_live(&text));
        assert!(!kimi_live(&text));
        assert!(!has_picker_cluster(&text));
        assert!(!has_choice_hint(&text));
    }

    #[test]
    fn live_chrome_guards_pass_on_real_screen_shapes() {
        assert!(pi_live(&pi_screen(&[])));
        assert!(pi_live(&pi_screen(&["\u{2800} Working..."])));
        assert!(!pi_live(&screen(&[])));

        assert!(kimi_live(&kimi_screen(&[])));
        assert!(kimi_live(&kimi_screen(&[
            "\u{1F311}\u{FE0F} \u{00B7} building"
        ])));
        assert!(kimi_live(&kimi_screen(&[
            "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            "  \u{2191}\u{2193} select  1/2 choose  \u{21B5} confirm",
        ])));
        let mut dialog = vec![
            "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            "  \u{2191}\u{2193} select  1/2 choose  \u{21B5} confirm",
            "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        ];
        dialog.insert(0, "conversation output");
        assert!(
            kimi_live(&dialog.join("\n")),
            "a dialog frame without the footer"
        );
        let real_dialog = "question\n\u{2192} [5] Other: custom answer\n\ntype answer  \u{21B5} save  tab switch  esc cancel";
        assert!(
            kimi_live(real_dialog),
            "the real captured dialog frame ends with the hint line"
        );
        let submit_dialog = "  [1] Submit\n  [2] Cancel\n  \u{2191}\u{2193} select  1/2 choose  \u{21B5} confirm  tab switch  esc cancel";
        assert!(
            kimi_live(submit_dialog),
            "a footer-less submit dialog is live via its own hint and options"
        );
        let approval_dialog = "  \u{25B6} 1. Allow\n    2. Deny\n  \u{2191}\u{2193} select \u{00B7} 1/2 choose \u{00B7} \u{21B5} confirm";
        assert!(
            kimi_live(approval_dialog),
            "a footer-less approval dialog is live via its own hint and rows"
        );
        assert!(
            !kimi_live(
                "older transcript line\n  \u{2191}\u{2193} select  1/2 choose  \u{21B5} confirm"
            ),
            "a panned frame ending in a stray hint without dialog rows must be rejected"
        );
        assert!(!kimi_live(&screen(&[])));
    }

    #[test]
    fn pi_live_tolerates_a_clipped_footer_but_not_an_empty_one() {
        let rule = "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";
        let mut frame = vec!["conversation output".to_string(), rule.to_string()];
        assert!(
            !pi_live(&frame.join("\n")),
            "a bare rule has no footer below it"
        );
        frame.push("/work/proj (main)".to_string());
        assert!(
            pi_live(&frame.join("\n")),
            "rule plus the cwd line is a clipped but live footer"
        );
        frame.push("$0.400 47.3%/1.0M (auto)".to_string());
        assert!(pi_live(&frame.join("\n")));
        for _ in 0..4 {
            frame.push("extra status line".to_string());
        }
        assert!(
            pi_live(&frame.join("\n")),
            "six lines below the rule stay live"
        );
        frame.push("one too many".to_string());
        assert!(
            !pi_live(&frame.join("\n")),
            "seven lines below the rule are not the footer"
        );
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
                    has_choice_hint(&text),
                    has_picker_cluster(&text),
                    has_numbered_choice(&text),
                    has_done_marker(&text),
                    kimi_working(&text),
                    kimi_questionnaire(&text),
                );
            }
        }
    }
}
