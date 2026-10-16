use qol_plugin_api::launcher_flows::{DetailText, FlowDetail};

use crate::app::rows::{escaped_len, fit, numbered, SIZE_FACT};
use crate::history::{Clip, ClipKind};

const HEAD_LINES: usize = 40;
const TAIL_LINES: usize = 12;
const LINE_BYTES: usize = 400;
const PROSE_LINE_BYTES: usize = 4 * 1024;
const HEAD_BYTES: usize = 24 * 1024;
const TAIL_BYTES: usize = 8 * 1024;
const ITEM_FRAMING: usize = 3;

pub fn build(clip: &Clip) -> FlowDetail {
    let Some(text) = clip.text.as_deref().filter(|_| shows_text(clip)) else {
        return FlowDetail::default();
    };
    FlowDetail {
        text: Some(excerpt(text, numbered(clip, text))),
        shown_fact: Some(SIZE_FACT.to_string()),
    }
}

fn shows_text(clip: &Clip) -> bool {
    !clip.is_secret()
        && clip.image.is_none()
        && !matches!(clip.kind, ClipKind::Link | ClipKind::Color)
}

fn excerpt(text: &str, numbered: bool) -> DetailText {
    let lines = text.lines().count();
    let line_bytes = if numbered {
        LINE_BYTES
    } else {
        PROSE_LINE_BYTES
    };
    let head = take_lines(text.lines(), HEAD_LINES, line_bytes, HEAD_BYTES);
    let rest = lines - head.len();
    let mut tail = if numbered && rest > 0 {
        take_lines(
            text.lines().rev().take(rest.min(TAIL_LINES)),
            TAIL_LINES,
            LINE_BYTES,
            TAIL_BYTES,
        )
    } else {
        Vec::new()
    };
    tail.reverse();
    DetailText {
        head,
        tail,
        lines,
        chars: text.chars().count(),
        numbered,
    }
}

fn take_lines<'a>(
    lines: impl Iterator<Item = &'a str>,
    most: usize,
    line_bytes: usize,
    budget: usize,
) -> Vec<String> {
    let mut used = 0;
    let mut taken = Vec::new();
    for line in lines.take(most) {
        let room = budget.saturating_sub(used + ITEM_FRAMING);
        if room == 0 {
            break;
        }
        let line = fit(line, line_bytes.min(room));
        used += escaped_len(line) + ITEM_FRAMING;
        taken.push(line.to_string());
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(kind: ClipKind, text: &str) -> Clip {
        Clip {
            id: "c_1".to_string(),
            kind,
            text: Some(text.to_string()),
            image: None,
            bytes: text.len() as u64,
            app: None,
            copied_at_ms: 0,
            pinned: false,
        }
    }

    fn log(lines: usize) -> String {
        (1..=lines)
            .map(|at| format!("l{at}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_long_log_sends_its_head_its_real_tail_and_its_counts() {
        let text = log(1_284);

        let detail = build(&clip(ClipKind::Text, &text));

        assert_eq!(detail.shown_fact.as_deref(), Some("Size"));
        let excerpt = detail.text.unwrap();
        assert!(excerpt.numbered);
        assert_eq!(excerpt.lines, 1_284);
        assert_eq!(excerpt.chars, text.chars().count());
        assert_eq!(excerpt.head.len(), HEAD_LINES);
        assert_eq!(excerpt.head[0], "l1");
        assert_eq!(excerpt.tail.len(), TAIL_LINES);
        assert_eq!(excerpt.tail.last().map(String::as_str), Some("l1284"));
        assert_eq!(excerpt.tail_start(), 1_284 - TAIL_LINES + 1);
        assert_eq!(excerpt.tail[0], format!("l{}", excerpt.tail_start()));
        assert_eq!(excerpt.hidden(), 1_284 - HEAD_LINES - TAIL_LINES);
    }

    #[test]
    fn a_short_clip_is_sent_whole_without_a_tail() {
        let text = log(6);

        let excerpt = build(&clip(ClipKind::Code, &text)).text.unwrap();

        assert_eq!(excerpt.head.len(), 6);
        assert!(excerpt.tail.is_empty());
        assert_eq!(excerpt.hidden(), 0);
        let just_over = log(HEAD_LINES + 3);
        let excerpt = build(&clip(ClipKind::Code, &just_over)).text.unwrap();
        assert_eq!(excerpt.tail.len(), 3);
        assert_eq!(excerpt.hidden(), 0);
    }

    #[test]
    fn prose_keeps_its_paragraphs_and_skips_the_tail() {
        let paragraph = "word ".repeat(10_000);
        let text = [paragraph.as_str(); 3].join("\n");

        let excerpt = build(&clip(ClipKind::Text, &text)).text.unwrap();

        assert!(!excerpt.numbered);
        assert!(excerpt.tail.is_empty());
        let sent: usize = excerpt.head.iter().map(|line| escaped_len(line)).sum();
        assert!(sent <= HEAD_BYTES, "{sent}");
        assert!(excerpt
            .head
            .iter()
            .all(|line| escaped_len(line) <= PROSE_LINE_BYTES));
        assert_eq!(excerpt.lines, 3);
    }

    #[test]
    fn secrets_images_links_and_colours_send_no_text() {
        let image = Clip {
            image: Some(crate::history::Image {
                blob: "c_1.png".to_string(),
                width: 1,
                height: 1,
            }),
            text: None,
            ..clip(ClipKind::Image, "")
        };
        for clip in [
            clip(ClipKind::Secret, "hunter2"),
            image,
            clip(ClipKind::Link, "https://example.com"),
            clip(ClipKind::Color, "#d9822b"),
        ] {
            assert_eq!(build(&clip), FlowDetail::default(), "{:?}", clip.kind);
        }
    }
}
