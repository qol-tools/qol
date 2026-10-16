use std::time::Duration;

use qol_plugin_api::launcher_flows::{thumb_url, Badge, FlowRow, FlowRows, Tag, Tone};

use crate::history::{swatch, Clip, ClipKind, Store};

pub const ROW_LIMIT: usize = 50;
pub const SIZE_FACT: &str = "Size";
const TITLE_BYTES: usize = 160;
const APP_BYTES: usize = 48;
const INLINE_COPY_BYTES: usize = 256;
const LINE_LIST_MIN: usize = 4;
const LINE_LIST_WIDTH: usize = 80;
const PAUSED: &str = "Capture is paused";

pub struct RowContext<'a> {
    pub now: u64,
    pub secret_ttl: Duration,
    pub store: &'a Store,
}

pub fn build(shown: &[Clip], paused: bool, context: &RowContext) -> FlowRows {
    FlowRows {
        rows: shown.iter().map(|clip| row(clip, context)).collect(),
        status: paused.then(|| Badge {
            text: PAUSED.to_string(),
            tone: Some(Tone::Warning),
        }),
    }
}

pub fn row(clip: &Clip, context: &RowContext) -> FlowRow {
    let secret = clip.is_secret();
    FlowRow {
        title: title(clip),
        id: Some(clip.id.clone()),
        kind: Some(clip.kind.id().to_string()),
        subtitle: Some(subtitle(clip, context.now)),
        copy: clip
            .text
            .as_deref()
            .filter(|text| !secret && escaped_len(text) <= INLINE_COPY_BYTES)
            .map(str::to_string),
        copy_image: clip
            .image
            .as_ref()
            .map(|image| context.store.blob_path(&image.blob)),
        mono: clip.kind.mono(),
        group: None,
        tags: tags(clip),
        preview: None,
        facts: facts(clip, context.now),
        thumb: clip
            .image
            .as_ref()
            .filter(|_| !secret)
            .map(|image| thumb_url(&context.store.blob_path(&image.blob))),
        swatch: (clip.kind == ClipKind::Color)
            .then(|| clip.text.as_deref().and_then(swatch))
            .flatten(),
        masked: secret,
        badge: secret
            .then(|| crate::secrets::remaining(clip.copied_at_ms, context.now, context.secret_ttl))
            .flatten()
            .map(|left| Badge {
                text: crate::secrets::countdown(left),
                tone: Some(Tone::Warning),
            }),
    }
}

pub fn title(clip: &Clip) -> String {
    if clip.is_secret() {
        return crate::secrets::mask().to_string();
    }
    if let Some(image) = &clip.image {
        return format!("Image {} × {}", image.width, image.height);
    }
    let text = clip.text.as_deref().unwrap_or_default();
    if clip.kind == ClipKind::File {
        return files_title(text);
    }
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    fit(line, TITLE_BYTES).to_string()
}

fn files_title(text: &str) -> String {
    let paths: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    let name = |path: &str| {
        let name = std::path::Path::new(path.trim_start_matches("file://"))
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        fit(&name, TITLE_BYTES).to_string()
    };
    match paths.as_slice() {
        [] => String::new(),
        [only] => name(only),
        [first, rest @ ..] => format!("{} and {} more", name(first), rest.len()),
    }
}

fn subtitle(clip: &Clip, now: u64) -> String {
    let mut parts = vec![clip.kind.label().to_string()];
    if let Some(app) = app(clip) {
        parts.push(app);
    }
    parts.push(age(now.saturating_sub(clip.copied_at_ms)));
    parts.join(" · ")
}

fn app(clip: &Clip) -> Option<String> {
    clip.app
        .as_deref()
        .map(|app| fit(app, APP_BYTES).to_string())
}

fn tags(clip: &Clip) -> Vec<Tag> {
    if !clip.pinned {
        return Vec::new();
    }
    vec![Tag {
        text: "Pinned".to_string(),
        tone: None,
    }]
}

pub fn numbered(clip: &Clip, text: &str) -> bool {
    clip.kind.mono() || is_line_list(text)
}

// Logs and listings read by line number; prose arrives as long paragraph lines.
fn is_line_list(text: &str) -> bool {
    let lines = text.lines().filter(|line| !line.trim().is_empty());
    let (count, chars) = lines.fold((0, 0), |(count, chars), line| {
        (count + 1, chars + line.chars().count())
    });
    count >= LINE_LIST_MIN && chars / count <= LINE_LIST_WIDTH
}

pub fn fit(text: &str, budget: usize) -> &str {
    let mut used = 0;
    for (at, character) in text.char_indices() {
        used += escaped_char_len(character);
        if used > budget {
            return &text[..at];
        }
    }
    text
}

pub fn escaped_len(text: &str) -> usize {
    text.chars().map(escaped_char_len).sum()
}

fn escaped_char_len(character: char) -> usize {
    match character {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        control if (control as u32) < 0x20 => 6,
        other => other.len_utf8(),
    }
}

fn facts(clip: &Clip, now: u64) -> Vec<(String, String)> {
    vec![
        ("Kind".to_string(), clip.kind.label().to_string()),
        (
            "From".to_string(),
            app(clip).unwrap_or_else(|| "Unknown app".to_string()),
        ),
        (SIZE_FACT.to_string(), size(clip)),
        (
            "Copied".to_string(),
            copied(now.saturating_sub(clip.copied_at_ms)),
        ),
    ]
}

pub fn size(clip: &Clip) -> String {
    if clip.is_secret() {
        return "Hidden".to_string();
    }
    if let Some(image) = &clip.image {
        return format!(
            "{} × {} · PNG · {}",
            image.width,
            image.height,
            bytes(clip.bytes)
        );
    }
    let text = clip.text.as_deref().unwrap_or_default();
    if clip.kind == ClipKind::File {
        return count(
            text.lines().filter(|line| !line.is_empty()).count(),
            "file",
            "files",
        );
    }
    let lines = text.lines().count();
    if lines > 1 {
        return format!("{} · {}", count(lines, "line", "lines"), bytes(clip.bytes));
    }
    count(text.chars().count(), "character", "characters")
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!(
        "{} {}",
        thousands(n as u64),
        if n == 1 { one } else { many }
    )
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

pub fn bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    if n < KB {
        return format!("{n} B");
    }
    if n < MB {
        return format!("{} KB", (n + KB / 2) / KB);
    }
    format!("{:.1} MB", n as f64 / MB as f64)
}

pub fn age(elapsed_ms: u64) -> String {
    let seconds = elapsed_ms / 1000;
    match seconds {
        0..=59 => format!("{seconds} s"),
        60..=3_599 => format!("{} min", seconds / 60),
        3_600..=86_399 => format!("{} h", seconds / 3_600),
        _ => format!("{} d", seconds / 86_400),
    }
}

fn copied(elapsed_ms: u64) -> String {
    if elapsed_ms < 5_000 {
        return "just now".to_string();
    }
    format!("{} ago", age(elapsed_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Image;

    const NOW: u64 = 1_800_000_000_000;

    fn clip(kind: ClipKind, text: &str, age_ms: u64) -> Clip {
        Clip {
            id: "c_0123456789abcdef".to_string(),
            kind,
            text: Some(text.to_string()),
            image: None,
            bytes: text.len() as u64,
            app: Some("Firefox".to_string()),
            copied_at_ms: NOW - age_ms,
            pinned: false,
        }
    }

    fn context(store: &Store) -> RowContext<'_> {
        RowContext {
            now: NOW,
            secret_ttl: Duration::from_secs(30),
            store,
        }
    }

    fn fact<'a>(row: &'a FlowRow, name: &str) -> &'a str {
        row.facts
            .iter()
            .find(|(label, _)| label == name)
            .map(|(_, value)| value.as_str())
            .unwrap_or_else(|| panic!("missing fact {name}"))
    }

    #[test]
    fn a_link_row_fills_the_schema() {
        let store = Store::at("/data/qol-clipboard");
        let link = clip(
            ClipKind::Link,
            "https://github.com/qol-tools/qol-monorepo/pull/147",
            120_000,
        );
        let row = row(&link, &context(&store));
        assert_eq!(row.id.as_deref(), Some("c_0123456789abcdef"));
        assert_eq!(row.kind.as_deref(), Some("link"));
        assert_eq!(
            row.title,
            "https://github.com/qol-tools/qol-monorepo/pull/147"
        );
        assert_eq!(row.subtitle.as_deref(), Some("Link · Firefox · 2 min"));
        assert_eq!(row.copy.as_deref(), Some(link.text.as_deref().unwrap()));
        assert!(row.mono);
        assert!(!row.masked);
        assert!(row.badge.is_none());
        assert!(row.thumb.is_none());
        assert!(row.tags.is_empty());
        let labels: Vec<&str> = row.facts.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, vec!["Kind", "From", "Size", "Copied"]);
        assert_eq!(fact(&row, "Copied"), "2 min ago");
        assert_eq!(fact(&row, "Size"), "50 characters");
        assert!(row.preview.is_none());
    }

    #[test]
    fn the_size_fact_carries_the_count() {
        let long = "let x = 1;\n".repeat(1_284);
        let mut code = clip(ClipKind::Code, &long, 0);
        code.bytes = 48 * 1024;
        let cases = [
            (code, "1,284 lines · 48 KB"),
            (clip(ClipKind::Text, &"a".repeat(612), 0), "612 characters"),
            (clip(ClipKind::File, "/tmp/a\n/tmp/b\n/tmp/c", 0), "3 files"),
            (clip(ClipKind::Secret, "hunter2", 0), "Hidden"),
        ];
        for (clip, expected) in cases {
            assert_eq!(size(&clip), expected, "kind: {:?}", clip.kind);
        }
    }

    #[test]
    fn an_image_row_has_a_thumb_and_copies_the_blob() {
        let store = Store::at("/data/qol-clipboard");
        let image = Clip {
            kind: ClipKind::Image,
            text: None,
            image: Some(Image {
                blob: "c_1.png".to_string(),
                width: 856,
                height: 626,
            }),
            bytes: 13 * 1024,
            ..clip(ClipKind::Image, "", 0)
        };
        let row = row(&image, &context(&store));
        assert_eq!(row.title, "Image 856 × 626");
        assert_eq!(fact(&row, "Size"), "856 × 626 · PNG · 13 KB");
        assert_eq!(
            row.thumb.as_deref(),
            Some("file:///data/qol-clipboard/blobs/c_1.png")
        );
        assert_eq!(
            row.copy_image,
            Some(std::path::PathBuf::from(
                "/data/qol-clipboard/blobs/c_1.png"
            ))
        );
        assert!(row.copy.is_none());
        assert!(row.preview.is_none());
    }

    #[test]
    fn a_secret_row_is_masked_with_a_countdown() {
        let store = Store::at("/data/qol-clipboard");
        let secret = Clip {
            app: Some("KeePassXC".to_string()),
            ..clip(ClipKind::Secret, "hunter2", 8_000)
        };
        let row = row(&secret, &context(&store));
        assert!(row.masked);
        assert_eq!(row.title, "••••••••");
        assert!(!row.title.contains("hunter2"));
        assert_eq!(row.subtitle.as_deref(), Some("Secret · KeePassXC · 8 s"));
        assert_eq!(
            row.badge,
            Some(Badge {
                text: "clears in 22 s".to_string(),
                tone: Some(Tone::Warning)
            })
        );
        assert!(row.thumb.is_none());
        assert!(!row
            .preview
            .as_ref()
            .is_some_and(|preview| preview.text.contains("hunter2")));
        assert_eq!(row.copy, None);
    }

    #[test]
    fn colour_and_pinned_rows_carry_swatch_and_tag() {
        let store = Store::at("/data/qol-clipboard");
        let colour = Clip {
            pinned: true,
            ..clip(ClipKind::Color, "#D9822B", 0)
        };
        let row = row(&colour, &context(&store));
        assert_eq!(row.swatch.as_deref(), Some("#d9822b"));
        assert_eq!(
            row.tags,
            vec![Tag {
                text: "Pinned".to_string(),
                tone: None
            }]
        );
    }

    #[test]
    fn status_only_reports_a_paused_capture() {
        let store = Store::at("/data/qol-clipboard");
        let rows = build(&[clip(ClipKind::Text, "a", 0)], false, &context(&store));
        assert_eq!(rows.status, None);
        let rows = build(&[], true, &context(&store));
        assert_eq!(
            rows.status,
            Some(Badge {
                text: "Capture is paused".to_string(),
                tone: Some(Tone::Warning)
            })
        );
    }

    #[test]
    fn rows_serialize_as_the_v1_wire_shape() {
        let store = Store::at("/data/qol-clipboard");
        let rows = build(
            &[clip(ClipKind::Text, "hello\nworld", 0)],
            true,
            &context(&store),
        );
        let wire = serde_json::to_value(&rows).unwrap();
        assert_eq!(wire["rows"][0]["title"], "hello");
        assert_eq!(
            wire["rows"][0]["facts"][0],
            serde_json::json!(["Kind", "Text"])
        );
        assert_eq!(wire["status"]["tone"], "warning");
        let parsed: FlowRows = serde_json::from_value(wire).unwrap();
        assert_eq!(parsed, rows);
    }

    #[test]
    fn formatting_tables() {
        let ages = [
            (0, "0 s"),
            (59_999, "59 s"),
            (120_000, "2 min"),
            (3 * 3_600_000, "3 h"),
            (2 * 86_400_000, "2 d"),
        ];
        for (elapsed, label) in ages {
            assert_eq!(age(elapsed), label);
        }
        let sizes = [(612, "612 B"), (49_152, "48 KB"), (1_572_864, "1.5 MB")];
        for (n, label) in sizes {
            assert_eq!(bytes(n), label);
        }
        assert_eq!(thousands(1_284), "1,284");
        assert_eq!(thousands(1_000_000), "1,000,000");
        assert_eq!(thousands(999), "999");
        assert_eq!(copied(1_000), "just now");
    }

    #[test]
    fn fit_counts_the_bytes_json_escaping_writes() {
        let cases = [
            ("short", 160, "short"),
            ("abcdef", 4, "abcd"),
            ("a\"b", 3, "a\""),
            ("a\"b", 2, "a"),
            ("\u{1}\u{1}", 6, "\u{1}"),
            ("éé", 3, "é"),
            ("", 0, ""),
        ];
        for (text, budget, expected) in cases {
            assert_eq!(fit(text, budget), expected, "{text:?} in {budget}");
            assert!(escaped_len(fit(text, budget)) <= budget);
        }
        assert_eq!(escaped_len("a\n\u{1}é"), 1 + 2 + 6 + 2);
        let wire = serde_json::to_string("a\n\u{1}é\"\\\t").unwrap();
        assert_eq!(escaped_len("a\n\u{1}é\"\\\t") + 2, wire.len());
    }

    #[test]
    fn a_long_log_row_carries_neither_its_text_nor_a_preview() {
        let store = Store::at("/data/qol-clipboard");
        let log = (1..=1_284)
            .map(|at| format!("l{at}"))
            .collect::<Vec<_>>()
            .join("\n");
        let row = row(&clip(ClipKind::Text, &log, 0), &context(&store));
        assert_eq!(row.title, "l1");
        assert_eq!(row.copy, None);
        assert_eq!(row.preview, None);
        assert_eq!(fact(&row, "Size"), "1,284 lines · 6 KB");
    }

    #[test]
    fn a_small_clip_keeps_its_text_inline() {
        let store = Store::at("/data/qol-clipboard");
        let small = "x".repeat(INLINE_COPY_BYTES);
        let row = row(&clip(ClipKind::Text, &small, 0), &context(&store));
        assert_eq!(row.copy.as_deref(), Some(small.as_str()));
        let over = "x".repeat(INLINE_COPY_BYTES + 1);
        let row = super::row(&clip(ClipKind::Text, &over, 0), &context(&store));
        assert_eq!(row.copy, None);
    }

    #[test]
    fn logs_read_by_number_and_prose_does_not() {
        let paragraph = "Quick update on the display work: mode changes now save through Muffin on Cinnamon, so the refresh rate sticks after a restart.";
        let email = [paragraph; 4].join("\n\n");
        let prose = clip(ClipKind::Text, &email, 0);
        assert!(!numbered(&prose, &email));
        let log = "a\nb\nc\nd\ne";
        assert!(numbered(&clip(ClipKind::Text, log, 0), log));
        assert!(numbered(&clip(ClipKind::Code, "x", 0), "x"));
    }
}
