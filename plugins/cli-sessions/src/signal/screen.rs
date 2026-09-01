use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use qol_terminal_sessions::cli::{CliTool, KIMI_TOOL_ID, PI_TOOL_ID};

pub fn screen_hash(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

struct ScreenStabilizer {
    apply: fn(&str) -> Cow<'_, str>,
}

impl ScreenStabilizer {
    fn stabilize<'a>(&self, text: &'a str) -> Cow<'a, str> {
        (self.apply)(text)
    }
}

const PI: ScreenStabilizer = ScreenStabilizer { apply: pi_stable };
const KIMI: ScreenStabilizer = ScreenStabilizer { apply: kimi_stable };
const IDENTITY: ScreenStabilizer = ScreenStabilizer {
    apply: identity_stable,
};

fn identity_stable(text: &str) -> Cow<'_, str> {
    Cow::Borrowed(text)
}

pub fn stable_screen<'a>(text: &'a str, tool: &CliTool) -> Cow<'a, str> {
    let stabilizer = match tool.id.as_str() {
        PI_TOOL_ID => &PI,
        KIMI_TOOL_ID => &KIMI,
        _ => &IDENTITY,
    };
    stabilizer.stabilize(text)
}

const PI_FOOTER_BELOW_MAX: usize = 6;

fn pi_stable(text: &str) -> Cow<'_, str> {
    let lines: Vec<&str> = text.lines().collect();
    let is_rule = |line: &str| {
        !line.trim().is_empty() && line.trim().chars().all(|character| character == '\u{2500}')
    };
    let Some(border) = lines.iter().rposition(|line| is_rule(line)) else {
        return Cow::Borrowed(text);
    };
    let below = lines[border + 1..]
        .iter()
        .filter(|line| !line.trim().is_empty())
        .count();
    if below > PI_FOOTER_BELOW_MAX {
        return Cow::Borrowed(text);
    }
    let end = lines[..border].iter().map(|line| line.len() + 1).sum();
    Cow::Borrowed(text.get(..end).unwrap_or(text))
}

fn kimi_stable(text: &str) -> Cow<'_, str> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(box_top) = lines
        .iter()
        .rposition(|line| line.trim_start().starts_with('\u{256D}'))
    else {
        return Cow::Borrowed(text);
    };
    if lines.len().saturating_sub(box_top) > 6 {
        return Cow::Borrowed(text);
    }
    let end = lines[..box_top].iter().map(|line| line.len() + 1).sum();
    Cow::Borrowed(text.get(..end).unwrap_or(text))
}

#[cfg(test)]
mod tests {
    use super::{screen_hash, stable_screen};
    use qol_terminal_sessions::cli::{claude_tool, codex_tool, generic_tool, kimi_tool, pi_tool};

    #[test]
    fn screen_hash_changes_with_content_and_is_stable_for_equal_text() {
        assert_eq!(screen_hash("same"), screen_hash("same"));
        assert_ne!(screen_hash("a"), screen_hash("b"));
    }

    #[test]
    fn pi_stable_ignores_footer_counter_changes() {
        let rule = "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";
        let base = format!(
            "conversation output\n\u{280B} Working...\n\n{rule}\n\n{rule}\n/tmp\n$0.000 (sub) 0.0%/262k (auto)"
        );
        let footer = format!(
            "conversation output\n\u{280B} Working...\n\n{rule}\n\n{rule}\n/tmp\n$0.480 (sub) 30.1%/1.0M (auto)"
        );
        let content = format!(
            "new output arrived\n\u{280B} Working...\n\n{rule}\n\n{rule}\n/tmp\n$0.000 (sub) 0.0%/262k (auto)"
        );
        let hash = |text: &str| screen_hash(stable_screen(text, &pi_tool()).as_ref());
        assert_eq!(
            hash(&base),
            hash(&footer),
            "footer counters must not count as movement"
        );
        assert_ne!(
            hash(&base),
            hash(&content),
            "content changes must count as movement"
        );
    }

    #[test]
    fn pi_stable_trims_a_footer_whose_draft_sits_above_the_border() {
        let rule = "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";
        let build = |cost: &str, draft: &str| {
            format!("conversation output\n{rule}\n{draft}\n{rule}\n/tmp\n{cost}")
        };
        let hash = |text: &str| screen_hash(stable_screen(text, &pi_tool()).as_ref());
        let base = build("$0.000 (sub) 0.0%/262k (auto)", "QOL_BRIDGE_DONE_marker");
        let bumped = build("$0.480 (sub) 30.1%/1.0M (auto)", "QOL_BRIDGE_DONE_marker");
        let changed = build("$0.000 (sub) 0.0%/262k (auto)", "a different draft");
        assert_eq!(
            hash(&base),
            hash(&bumped),
            "footer counters must not count as movement"
        );
        assert_ne!(
            hash(&base),
            hash(&changed),
            "content above the border must count as movement"
        );
    }

    #[test]
    fn pi_stable_requires_footer_rules_near_the_tail() {
        let rule = "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";
        let a = format!(
            "streamed output\n{rule}\nchanging line A\n\n{rule}\n\n{rule}\n/tmp\n$0.000 (sub) 0.0%/262k (auto)"
        );
        let b = format!(
            "streamed output\n{rule}\nchanging line B\n\n{rule}\n\n{rule}\n/tmp\n$0.000 (sub) 0.0%/262k (auto)"
        );
        let hash = |text: &str| screen_hash(stable_screen(text, &pi_tool()).as_ref());
        assert_ne!(
            hash(&a),
            hash(&b),
            "a rule-looking line inside streamed output must not hide movement below it"
        );
    }

    #[test]
    fn kimi_stable_ignores_status_bar_changes() {
        let boxed = "\u{256D}\u{2500}\u{2500}\u{2500}\u{256E}\n\u{2502} >  \u{2502}\n\u{2570}\u{2500}\u{2500}\u{2500}\u{256F}";
        let base = format!(
            "conversation output\n{boxed}\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]\ncontext: 17% (41.1k/256k)"
        );
        let status = format!(
            "conversation output\n{boxed}\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]\ncontext: 21% (51.5k/256k)"
        );
        let content = format!(
            "new output arrived\n{boxed}\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]\ncontext: 17% (41.1k/256k)"
        );
        let hash = |text: &str| screen_hash(stable_screen(text, &kimi_tool()).as_ref());
        assert_eq!(
            hash(&base),
            hash(&status),
            "status bar changes must not count as movement"
        );
        assert_ne!(
            hash(&base),
            hash(&content),
            "content changes must count as movement"
        );
    }

    #[test]
    fn non_tool_screens_are_never_normalized() {
        let text = "plain output stays as-is";
        assert_eq!(stable_screen(text, &claude_tool()).as_ref(), text);
        assert_eq!(stable_screen(text, &codex_tool()).as_ref(), text);
        assert_eq!(stable_screen(text, &generic_tool()).as_ref(), text);
    }
}
