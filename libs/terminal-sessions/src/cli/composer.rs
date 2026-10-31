use super::CliComposer;

const RULE: char = '\u{2500}';
const LABELED_RULE_MIN_RUN: usize = 3;
const CLAUDE_GLYPHS: [char; 2] = ['\u{276F}', '>'];
const CLAUDE_CHIPS: [Chip; 1] = [Chip::new("[Pasted text #", "]")];
const PI_CHIPS: [Chip; 1] = [Chip::new("[paste #", "]")];
const CODEX_GLYPH: char = '\u{203A}';
const CODEX_CHIPS: [Chip; 1] = [Chip::new("[Pasted Content ", " chars]")];
const KIMI_GLYPH: char = '>';
const BOX_SIDE: char = '\u{2502}';
const BOX_BOTTOM: char = '\u{2570}';

#[derive(Clone, Copy)]
struct Chip {
    open: &'static str,
    close: &'static str,
}

impl Chip {
    const fn new(open: &'static str, close: &'static str) -> Self {
        Self { open, close }
    }
}

#[derive(Clone, Copy)]
enum Frame {
    Open,
    Boxed,
}

impl Frame {
    fn inner(self, line: &str) -> &str {
        match self {
            Self::Open => line,
            Self::Boxed => {
                let line = line.trim();
                let line = line.strip_prefix(BOX_SIDE).unwrap_or(line);
                line.trim_end().strip_suffix(BOX_SIDE).unwrap_or(line)
            }
        }
    }

    fn closes(self, line: &str) -> bool {
        match self {
            Self::Open => line.trim().is_empty(),
            Self::Boxed => line.trim_start().starts_with(BOX_BOTTOM),
        }
    }
}

pub(crate) fn claude(screen: &str) -> CliComposer {
    ruled(screen, &CLAUDE_GLYPHS, &CLAUDE_CHIPS)
}

pub(crate) fn pi(screen: &str) -> CliComposer {
    ruled(screen, &[], &PI_CHIPS)
}

pub(crate) fn codex(screen: &str) -> CliComposer {
    marked(screen, CODEX_GLYPH, Frame::Open, &CODEX_CHIPS)
}

pub(crate) fn kimi(screen: &str) -> CliComposer {
    marked(screen, KIMI_GLYPH, Frame::Boxed, &[])
}

fn ruled(screen: &str, glyphs: &[char], chips: &[Chip]) -> CliComposer {
    let lines: Vec<&str> = screen.lines().collect();
    let Some(bottom) = lines.iter().rposition(|line| is_rule(line)) else {
        return CliComposer::Unreadable;
    };
    let Some(top) = lines[..bottom].iter().rposition(|line| is_rule(line)) else {
        return CliComposer::Unreadable;
    };
    let body = &lines[top + 1..bottom];
    if glyphs.is_empty() {
        return draft(&body.join("\n"), chips);
    }
    let Some(first) = body.iter().position(|line| !line.trim().is_empty()) else {
        return CliComposer::Unreadable;
    };
    match after_glyph(body[first], glyphs) {
        Some(rest) => draft(
            &std::iter::once(rest)
                .chain(body[first + 1..].iter().copied())
                .collect::<Vec<_>>()
                .join("\n"),
            chips,
        ),
        None => draft(&body[first..].join("\n"), chips),
    }
}

fn marked(screen: &str, glyph: char, frame: Frame, chips: &[Chip]) -> CliComposer {
    let lines: Vec<&str> = screen.lines().map(|line| frame.inner(line)).collect();
    let Some(start) = lines
        .iter()
        .rposition(|line| after_glyph(line, &[glyph]).is_some())
    else {
        return CliComposer::Unreadable;
    };
    let first = after_glyph(lines[start], &[glyph]).unwrap_or_default();
    let text = std::iter::once(first)
        .chain(
            lines[start + 1..]
                .iter()
                .take_while(|line| !frame.closes(line))
                .copied(),
        )
        .collect::<Vec<_>>()
        .join("\n");
    draft(&text, chips)
}

fn is_rule(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let label = trimmed.trim_start_matches(RULE);
    if label.is_empty() {
        return true;
    }
    let leading = trimmed.chars().count() - label.chars().count();
    leading >= LABELED_RULE_MIN_RUN && label.ends_with(RULE)
}

fn after_glyph<'a>(line: &'a str, glyphs: &[char]) -> Option<&'a str> {
    line.trim_start()
        .strip_prefix(glyphs)
        .filter(|rest| rest.chars().next().is_none_or(char::is_whitespace))
}

fn draft(text: &str, chips: &[Chip]) -> CliComposer {
    let (text, pastes) = without_chips(text, chips);
    let text = text.trim();
    if pastes == 0 && text.is_empty() {
        return CliComposer::Empty;
    }
    CliComposer::Draft {
        text: text.to_owned(),
        pastes,
    }
}

fn without_chips(text: &str, chips: &[Chip]) -> (String, usize) {
    let mut kept = String::with_capacity(text.len());
    let mut pastes = 0;
    let mut rest = text;
    'scan: while !rest.is_empty() {
        for chip in chips {
            if let Some(after) = rest.strip_prefix(chip.open) {
                if let Some(end) = after.find(chip.close) {
                    if after[..end].chars().all(chip_label) {
                        pastes += 1;
                        rest = &after[end + chip.close.len()..];
                        continue 'scan;
                    }
                }
            }
        }
        let mut characters = rest.chars();
        if let Some(character) = characters.next() {
            kept.push(character);
        }
        rest = characters.as_str();
    }
    (kept, pastes)
}

fn chip_label(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == ' ' || character == '+'
}

#[cfg(test)]
mod tests {
    use super::{claude, codex, kimi, pi};
    use crate::cli::CliComposer;

    fn draft(text: &str, pastes: usize) -> CliComposer {
        CliComposer::Draft {
            text: text.to_owned(),
            pastes,
        }
    }

    const CLAUDE_RULE: &str = "────────────────────────────────────────";

    fn claude_screen(body: &str) -> String {
        format!(
            "\u{276F} earlier prompt\n\u{25CF} Done.\n\n{CLAUDE_RULE}\n{body}\n{CLAUDE_RULE}\n  /work \u{2502} Opus 5.5 high\n  \u{23F5}\u{23F5} bypass permissions on (shift+tab to cycle)"
        )
    }

    #[test]
    fn claude_reads_its_ruled_input_box() {
        let cases = [
            ("\u{276F}\u{a0}", CliComposer::Empty),
            ("\u{276F} ", CliComposer::Empty),
            ("> ", CliComposer::Empty),
            ("\u{276F}\u{a0}hello wor", draft("hello wor", 0)),
            (
                "\u{276F} first line\n  second line",
                draft("first line\n  second line", 0),
            ),
            ("! ls -la", draft("! ls -la", 0)),
            ("!", draft("!", 0)),
            ("\u{276F} [Pasted text #1 +11 lines]", draft("", 1)),
            ("! [Pasted text #1 +11 lines]", draft("!", 1)),
            ("\u{276F} x[Pasted text #2]", draft("x", 1)),
            ("", CliComposer::Unreadable),
        ];
        for (body, expected) in cases {
            assert_eq!(claude(&claude_screen(body)), expected, "{body:?}");
        }
        assert_eq!(
            claude("Do you want to proceed?\n\u{276F} 1. Yes\n  2. No"),
            CliComposer::Unreadable
        );
    }

    const LABELED_RULE: &str = "──────────────────────────────────────── ultracode ─";

    fn framed_screen(top: &str, body: &str, bottom: &str) -> String {
        format!(
            "\u{276F} earlier prompt\n\u{25CF} Done.\n\n{top}\n{body}\n{bottom}\n  /work \u{2502} Opus 5.5 high"
        )
    }

    #[test]
    fn claude_reads_a_box_whose_top_rule_carries_a_label() {
        let empty = framed_screen(LABELED_RULE, "\u{276F}\u{a0}", CLAUDE_RULE);
        assert_eq!(claude(&empty), CliComposer::Empty);
        let typed = framed_screen(LABELED_RULE, "\u{276F} hello wor", CLAUDE_RULE);
        assert_eq!(claude(&typed), draft("hello wor", 0));
    }

    #[test]
    fn claude_reads_a_box_whose_bottom_rule_carries_a_label() {
        let empty = framed_screen(CLAUDE_RULE, "\u{276F}\u{a0}", LABELED_RULE);
        assert_eq!(claude(&empty), CliComposer::Empty);
        let typed = framed_screen(CLAUDE_RULE, "\u{276F} hello wor", LABELED_RULE);
        assert_eq!(claude(&typed), draft("hello wor", 0));
        let both = framed_screen(LABELED_RULE, "\u{276F}\u{a0}", LABELED_RULE);
        assert_eq!(claude(&both), CliComposer::Empty);
    }

    #[test]
    fn a_prose_line_starting_with_a_rule_character_is_not_a_rule() {
        let lines = [
            "\u{2500} the box is above",
            "\u{2500}\u{2500} label \u{2500}",
            "\u{2500}\u{2500}\u{2500} label",
        ];
        for line in lines {
            let screen = framed_screen(CLAUDE_RULE, "\u{276F}\u{a0}", line);
            assert_eq!(claude(&screen), CliComposer::Unreadable, "{line:?}");
        }
    }

    #[test]
    fn codex_reads_the_marked_line_down_to_the_blank_line() {
        let footer = "\n \n  GPT-6.1-Sol medium fast \u{b7} /work\n  \u{2190} for agents";
        let cases = [
            ("\u{203A} ", CliComposer::Empty),
            ("\u{203A} hello wor", draft("hello wor", 0)),
            (
                "\u{203A} hello wor line one\n  line two",
                draft("hello wor line one\n  line two", 0),
            ),
            (
                "\u{203A} [Pasted Content 2048 chars]row 49\n  row 50",
                draft("row 49\n  row 50", 1),
            ),
        ];
        for (composer, expected) in cases {
            let screen =
                format!("\u{203A} an earlier prompt\n\n\u{2022} Done.\n\n{composer}{footer}");
            assert_eq!(codex(&screen), expected, "{composer:?}");
        }
        assert_eq!(
            codex("  Hooks need review\n\u{203A} 1. Review hooks\n  2. Trust all and continue\n\n  enter confirm"),
            draft("1. Review hooks\n  2. Trust all and continue", 0)
        );
        assert_eq!(codex("$ "), CliComposer::Unreadable);
    }

    #[test]
    fn pi_reads_the_box_between_its_last_two_rules() {
        let rule = "──────────────────────────────";
        let screen = |body: &str| {
            format!(" Packages:\n{rule}\n qol-memory: 1 question\n{rule}\n{body}\n{rule}\n/work\nctx 0.0%")
        };
        assert_eq!(pi(&screen("")), CliComposer::Empty);
        assert_eq!(pi(&screen("   ")), CliComposer::Empty);
        assert_eq!(pi(&screen("hello wor")), draft("hello wor", 0));
        assert_eq!(pi(&screen("[paste #1 +12 lines]")), draft("", 1));
        assert_eq!(pi("no editor yet"), CliComposer::Unreadable);
    }

    #[test]
    fn kimi_reads_its_framed_box() {
        let screen = |body: &str| {
            format!(
                " \u{256D}────────\u{256E}\n{body}\n \u{2570}────────\u{256F}\n K3 thinking: high"
            )
        };
        assert_eq!(kimi(&screen(" \u{2502} > ")), CliComposer::Empty);
        assert_eq!(
            kimi(&screen(" \u{2502} > hello wor \u{2502}")),
            draft("hello wor", 0)
        );
        assert_eq!(
            kimi(&screen(" \u{2502} > row 34\n \u{2502}   row 35")),
            draft("row 34\n   row 35", 0)
        );
        assert_eq!(kimi("Trust this folder?"), CliComposer::Unreadable);
    }

    #[test]
    fn a_draft_holds_only_what_was_sent_when_nothing_else_is_in_it() {
        let sent = "line 0 of a prompt\nline 1 of a prompt\nline 2 of a prompt";
        assert!(draft(sent, 0).holds_only(sent));
        assert!(draft("line 2 of a\n   prompt", 0).holds_only(sent));
        assert!(draft("", 1).holds_only(sent));
        assert!(draft("line 2 of a prompt", 1).holds_only(sent));
        assert!(!draft("line 0 of a prompt line 1 of a prompt", 0).holds_only(sent));
        assert!(!draft("x", 1).holds_only(sent));
        assert!(!draft("p", 1).holds_only(sent));
        assert!(!draft("p", 0).holds_only(sent));
        assert!(!draft("line 2 of a promptp", 1).holds_only(sent));
        assert!(!draft("xline 0 of a prompt", 0).holds_only(sent));
        assert!(!draft("line 2 of a prompty", 0).holds_only(sent));
        assert!(!draft("", 0).holds_only(sent));
        assert!(!CliComposer::Empty.holds_only(sent));
        assert!(!CliComposer::Unreadable.holds_only(sent));
    }
}
