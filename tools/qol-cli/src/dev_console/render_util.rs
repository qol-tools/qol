use std::cell::Cell;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use super::picker::{FILTER_PANEL_MAX_WIDTH, FILTER_PANEL_MIN_WIDTH};
use super::Dash;

thread_local! {
    static ACCENT: Cell<Color> = const { Cell::new(Color::Green) };
    static BOTTOM_RESERVED: Cell<u16> = const { Cell::new(0) };
}

pub(super) fn set_frame_accent(color: Color) {
    ACCENT.with(|cell| cell.set(color));
}

pub(super) fn reset_bottom_stack() {
    BOTTOM_RESERVED.with(|cell| cell.set(0));
}

pub(super) fn accent() -> Color {
    ACCENT.with(Cell::get)
}

pub(super) fn caret(selected: bool) -> Span<'static> {
    if selected {
        "▸ ".fg(accent()).bold()
    } else {
        "  ".into()
    }
}

pub(super) struct SignBox<'a> {
    pub(super) title: &'a str,
    pub(super) rows: Vec<Line<'a>>,
}

impl SignBox<'_> {
    pub(super) const CHROME_ROWS: u16 = 4;

    pub(super) fn capacity(height: u16) -> usize {
        height.saturating_sub(Self::CHROME_ROWS) as usize
    }

    pub(super) fn render(self, frame: &mut Frame, area: Rect, accent: Color) {
        let body = Rect {
            y: area.y + 1,
            height: area.height.saturating_sub(1),
            ..area
        };
        let block = Block::bordered().border_style(Style::new().fg(accent));
        let inner = block.inner(body);
        frame.render_widget(block, body);
        let rows_area = Rect {
            y: inner.y + 1,
            height: inner.height.saturating_sub(1),
            ..inner
        };
        frame.render_widget(Paragraph::new(self.rows), rows_area);
        Sign {
            content: Line::from(self.title.to_string().fg(accent).bold()),
        }
        .render(frame, body, accent);
    }
}

pub(super) fn panel_width(area: Rect) -> u16 {
    if area.width <= FILTER_PANEL_MIN_WIDTH {
        area.width
    } else {
        area.width
            .saturating_sub(4)
            .clamp(FILTER_PANEL_MIN_WIDTH, FILTER_PANEL_MAX_WIDTH)
    }
}

pub(super) fn render_bottom_panel(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    rows: Vec<Line<'static>>,
    accent: Color,
) {
    let width = panel_width(area);
    render_bottom_panel_with_width(frame, area, title, rows, accent, width);
}

fn render_bottom_panel_with_width(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    mut rows: Vec<Line<'static>>,
    accent: Color,
    width: u16,
) {
    let reserved = BOTTOM_RESERVED.with(Cell::get);
    let height =
        (rows.len() as u16 + SignBox::CHROME_ROWS).min(area.height.saturating_sub(reserved));
    if width == 0 || height == 0 {
        return;
    }
    rows.truncate(SignBox::capacity(height));
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + area.height.saturating_sub(height + 1 + reserved),
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    SignBox { title, rows }.render(frame, rect, accent);
    BOTTOM_RESERVED.with(|cell| cell.set(reserved + height));
}

pub(super) struct Sign {
    pub(super) content: Line<'static>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct NavigationOverflow {
    pub(super) above: bool,
    pub(super) below: bool,
}

impl NavigationOverflow {
    pub(super) fn from_window(start: usize, height: usize, total: usize) -> Self {
        Self {
            above: start > 0,
            below: start.saturating_add(height) < total,
        }
    }
}

const NAVIGATION_CUE_WIDTH: u16 = 5;
const NAVIGATION_CUE_HEIGHT: u16 = 3;
const NAVIGATION_CUE_GAP: u16 = 1;

impl Sign {
    pub(super) fn render_bottom(
        self,
        frame: &mut Frame,
        body: Rect,
        accent: Color,
        navigation: NavigationOverflow,
    ) {
        let width = self.sign_width();
        let x = body.x + (body.width - width) / 2;
        self.render_bottom_at(frame, body, x, width, accent, navigation);
    }

    pub(super) fn render_bottom_right(self, frame: &mut Frame, body: Rect, accent: Color) {
        let width = self.sign_width();
        let x = body.x + body.width.saturating_sub(width);
        self.render_bottom_at(frame, body, x, width, accent, NavigationOverflow::default());
    }

    fn sign_width(&self) -> u16 {
        self.content.width() as u16 + 4
    }

    fn render_bottom_at(
        self,
        frame: &mut Frame,
        body: Rect,
        x: u16,
        width: u16,
        accent: Color,
        navigation: NavigationOverflow,
    ) {
        let span = self.content.width() as u16 + 2;
        if width + 2 > body.width || body.height < 3 {
            return;
        }
        let y = body.y + body.height - 1;
        let bar = "─".repeat(span as usize);
        render_overlay(frame, x, y - 1, Line::from(format!("╭{bar}╮").fg(accent)));
        let mut middle = vec!["┤ ".fg(accent)];
        middle.extend(self.content.spans);
        middle.push(" ├".fg(accent));
        render_overlay(frame, x, y, Line::from(middle));
        if y + 1 < frame.area().y + frame.area().height {
            render_overlay(frame, x, y + 1, Line::from(format!("╰{bar}╯").fg(accent)));
        }
        render_navigation_flanks(frame, body, x, y - 1, width, accent, navigation);
    }

    pub(super) fn render(self, frame: &mut Frame, body: Rect, accent: Color) {
        let span = self.content.width() as u16 + 2;
        let width = span + 2;
        if width + 2 > body.width {
            return;
        }
        let x = body.x + (body.width - width) / 2;
        let bar = "─".repeat(span as usize);
        render_overlay(
            frame,
            x,
            body.y.saturating_sub(1),
            Line::from(format!("╭{bar}╮").fg(accent)),
        );
        let mut middle = vec!["┤ ".fg(accent)];
        middle.extend(self.content.spans);
        middle.push(" ├".fg(accent));
        render_overlay(frame, x, body.y, Line::from(middle));
        render_overlay(
            frame,
            x,
            body.y + 1,
            Line::from(format!("╰{bar}╯").fg(accent)),
        );
    }
}

fn render_navigation_flanks(
    frame: &mut Frame,
    body: Rect,
    sign_x: u16,
    y: u16,
    sign_width: u16,
    accent: Color,
    navigation: NavigationOverflow,
) {
    if navigation.below {
        let offset = NAVIGATION_CUE_WIDTH + NAVIGATION_CUE_GAP;
        if let Some(x) = sign_x.checked_sub(offset) {
            render_navigation_box(frame, body, x, y, accent, "v");
        }
    }
    if navigation.above {
        let x = sign_x
            .saturating_add(sign_width)
            .saturating_add(NAVIGATION_CUE_GAP);
        render_navigation_box(frame, body, x, y, accent, "^");
    }
}

fn render_navigation_box(
    frame: &mut Frame,
    body: Rect,
    x: u16,
    y: u16,
    accent: Color,
    glyph: &'static str,
) {
    let right = x.saturating_add(NAVIGATION_CUE_WIDTH);
    let body_right = body.x.saturating_add(body.width);
    if x <= body.x || right >= body_right {
        return;
    }
    let rows = vec![
        Line::from("┌───┐".fg(accent)),
        Line::from(vec![
            "│ ".fg(accent),
            glyph.fg(accent).bold(),
            " │".fg(accent),
        ]),
        Line::from("└───┘".fg(accent)),
    ];
    frame.render_widget(
        Paragraph::new(rows),
        Rect::new(x, y, NAVIGATION_CUE_WIDTH, NAVIGATION_CUE_HEIGHT),
    );
}

fn render_overlay(frame: &mut Frame, x: u16, y: u16, line: Line<'static>) {
    let width = line.width() as u16;
    frame.render_widget(Paragraph::new(line), Rect::new(x, y, width, 1));
}

pub(super) const ITEM_GAP: u16 = 1;

fn space_rows(rows: Vec<Line>, gap: u16) -> Vec<Line> {
    if gap == 0 || rows.len() <= 1 {
        return rows;
    }
    let last = rows.len() - 1;
    let mut spaced = Vec::with_capacity(rows.len() + last * gap as usize);
    for (index, row) in rows.into_iter().enumerate() {
        spaced.push(row);
        if index != last {
            spaced.extend((0..gap).map(|_| Line::from("")));
        }
    }
    spaced
}

pub(super) fn spaced_height(items: usize, gap: u16) -> u16 {
    items as u16 + items.saturating_sub(1) as u16 * gap
}

pub(super) fn list_capacity(height: u16) -> usize {
    (height as usize + ITEM_GAP as usize) / (1 + ITEM_GAP as usize)
}

pub(super) fn cursor_window_start(total: usize, height: usize, cursor: usize) -> usize {
    if height == 0 || total <= height {
        return 0;
    }
    let max_start = total - height;
    if cursor >= height {
        return (cursor + 1 - height).min(max_start);
    }
    0
}

pub(super) fn view_content(frame: &mut Frame, area: Rect, lines: Vec<Line>) {
    frame.render_widget(Paragraph::new(space_rows(lines, ITEM_GAP)), area);
}

pub(super) fn list_window(dash: &mut Dash, area: Rect, total: usize) -> (usize, usize) {
    let height = list_capacity(area.height);
    dash.log_height = height;
    dash.scroll_offset = super::clamp_offset(total, height, dash.scroll_offset);
    (
        super::window_start(total, height, dash.scroll_offset),
        height,
    )
}

pub(super) fn list_status(total: usize, scroll_offset: usize) -> String {
    let mode = if scroll_offset == 0 {
        "follow"
    } else {
        "scroll"
    };
    format!("{total} lines · {mode}")
}

fn display_width(text: &str) -> usize {
    Span::raw(text).width()
}

pub(super) fn ellipsize_line(line: Line<'static>, width: usize) -> Line<'static> {
    if width == 0 || line.width() <= width {
        return line;
    }
    let budget = width - 1;
    let mut used = 0;
    let mut spans = Vec::new();
    for span in line.spans {
        let span_width = span.width();
        if used + span_width <= budget {
            used += span_width;
            spans.push(span);
            continue;
        }
        let mut kept = String::new();
        for ch in span.content.chars() {
            let ch_width = display_width(&ch.to_string());
            if used + ch_width > budget {
                break;
            }
            used += ch_width;
            kept.push(ch);
        }
        spans.push(Span::styled(kept, span.style));
        break;
    }
    spans.push(Span::raw("…"));
    Line::from(spans)
}

pub(super) fn wrapped_rows(text: &str, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for source in text.lines() {
        if source.trim().is_empty() {
            rows.push(Line::from(""));
            continue;
        }
        let mut current = String::new();
        let mut used = 0;
        for word in source.split_whitespace() {
            let word_width = display_width(word);
            let gap = usize::from(!current.is_empty());
            if used + gap + word_width <= width {
                if gap == 1 {
                    current.push(' ');
                }
                current.push_str(word);
                used += gap + word_width;
                continue;
            }
            if !current.is_empty() {
                rows.push(Line::from(std::mem::take(&mut current)));
                used = 0;
            }
            if word_width <= width {
                current.push_str(word);
                used = word_width;
                continue;
            }
            for ch in word.chars() {
                let ch_width = display_width(&ch.to_string());
                if used + ch_width > width {
                    rows.push(Line::from(std::mem::take(&mut current)));
                    used = 0;
                }
                current.push(ch);
                used += ch_width;
            }
        }
        if !current.is_empty() {
            rows.push(Line::from(current));
        }
    }
    if rows.is_empty() {
        rows.push(Line::from(""));
    }
    rows
}

pub(super) fn styled_line(raw: &str) -> Line<'_> {
    use ansi_to_tui::IntoText;
    let Ok(text) = raw.into_text() else {
        return Line::from(raw);
    };
    text.lines
        .into_iter()
        .next()
        .unwrap_or_else(|| Line::from(raw))
}

pub(super) fn format_duration(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    let (minutes, seconds) = (total / 60, total % 60);
    if minutes >= 60 {
        return format!("{}h{:02}m", minutes / 60, minutes % 60);
    }
    format!("{minutes}m{seconds:02}s")
}

pub(super) fn relative_age(now_ms: u64, then_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(then_ms) / 1000;
    match seconds {
        0..=9 => "just now".to_string(),
        10..=59 => format!("{seconds}s ago"),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86399 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}

pub(super) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_window_keeps_selection_visible() {
        let cases = [
            (10, 4, 0, 0),
            (10, 4, 2, 0),
            (10, 4, 4, 1),
            (10, 4, 9, 6),
            (3, 5, 2, 0),
        ];
        for (total, height, cursor, expected) in cases {
            assert_eq!(
                cursor_window_start(total, height, cursor),
                expected,
                "total={total} height={height} cursor={cursor}"
            );
        }
    }

    #[test]
    fn ellipsize_line_truncates_by_display_cell() {
        let cases = [
            ("short", 10, "short"),
            ("exactly ten", 11, "exactly ten"),
            ("a longer message", 10, "a longer …"),
            ("日本語テスト", 5, "日本…"),
        ];
        for (input, width, expected) in cases {
            assert_eq!(
                ellipsize_line(Line::from(input.to_string()), width).to_string(),
                expected,
                "input: {input} width: {width}"
            );
        }
    }

    #[test]
    fn ellipsize_line_keeps_span_styles_on_the_kept_prefix() {
        let line = Line::from(vec!["head ".fg(Color::Yellow), "tail overflowing".into()]);
        let truncated = ellipsize_line(line, 10);
        assert_eq!(truncated.to_string(), "head tail…");
        assert_eq!(truncated.spans[0].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn wrapped_rows_wrap_by_word_and_hard_break_long_words() {
        let cases = [
            ("one two three", 8, vec!["one two", "three"]),
            ("word", 10, vec!["word"]),
            ("abcdefghij", 4, vec!["abcd", "efgh", "ij"]),
            ("line1\n\nline2", 10, vec!["line1", "", "line2"]),
            ("", 10, vec![""]),
        ];
        for (input, width, expected) in cases {
            let rows: Vec<String> = wrapped_rows(input, width)
                .iter()
                .map(Line::to_string)
                .collect();
            assert_eq!(rows, expected, "input: {input:?} width: {width}");
        }
    }

    #[test]
    fn relative_age_buckets_seconds_minutes_hours_days() {
        let cases = [
            (5_000, "just now"),
            (59_000, "59s ago"),
            (60_000, "1m ago"),
            (3_599_000, "59m ago"),
            (3_600_000, "1h ago"),
            (86_399_000, "23h ago"),
            (86_400_000, "1d ago"),
            (0, "just now"),
        ];
        for (elapsed_ms, expected) in cases {
            assert_eq!(
                relative_age(1_000_000_000 + elapsed_ms, 1_000_000_000),
                expected,
                "elapsed_ms: {elapsed_ms}"
            );
        }
    }

    #[test]
    fn relative_age_gains_a_seconds_bucket() {
        let cases = [
            (5_000, "just now"),
            (10_000, "10s ago"),
            (59_000, "59s ago"),
            (60_000, "1m ago"),
        ];
        for (elapsed_ms, expected) in cases {
            assert_eq!(
                relative_age(1_000_000_000 + elapsed_ms, 1_000_000_000),
                expected,
                "elapsed_ms: {elapsed_ms}"
            );
        }
    }
}
