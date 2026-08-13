use qol_diff::{HeatLevel, LineChange, LineKind, TokenKind};

pub const CANVAS_BG: u32 = 0x14181f;
pub const LIST_BG: u32 = 0x11141a;
pub const BORDER: u32 = 0x2f3644;
pub const GUTTER_TEXT: u32 = 0x4d5870;
pub const TEXT_PRIMARY: u32 = 0xd4dbea;
pub const TEXT_MUTED: u32 = 0x67748f;
pub const TEXT_SELECTED: u32 = 0xf8fbff;
pub const LIST_SELECTED_BG: u32 = 0x2f3644;
pub const ERROR_TEXT: u32 = 0xff6b6b;
pub const TEXT_ADDED: u32 = 0x4ade80;
pub const TEXT_REMOVED: u32 = 0xff6b6b;
const LINE_WARM: u32 = 0x241c12;
const LINE_WARM_DIMMED: u32 = 0x1d1b17;
const LINE_HOT: u32 = 0x33240f;
const LINE_HOT_DIMMED: u32 = 0x262019;
const TOKEN_WARM: u32 = 0x3d2e15;
const TOKEN_HOT: u32 = 0x57401a;
pub const TOKEN_IGNITE_FLASH: u32 = 0xffffff;
pub const TOKEN_MORPH_FLARE: u32 = 0x9a6a20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineStyle {
    pub background_heat: HeatLevel,
    pub dimmed: bool,
}

impl Default for LineStyle {
    fn default() -> Self {
        Self {
            background_heat: HeatLevel::Cool,
            dimmed: false,
        }
    }
}

pub struct CodeSurface {
    styles: Vec<LineStyle>,
    scroll_offset: usize,
    gutter_enabled: bool,
    gutter_width: usize,
}

impl Default for CodeSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeSurface {
    pub fn new() -> Self {
        Self {
            styles: Vec::new(),
            scroll_offset: 0,
            gutter_enabled: true,
            gutter_width: 0,
        }
    }

    pub fn set_lines(&mut self, lines: &[LineChange]) {
        self.styles = lines.iter().map(style_from_line).collect();
        self.gutter_width = if self.gutter_enabled {
            gutter_digit_width(lines)
        } else {
            0
        };
        self.scroll_offset = 0;
    }

    pub fn line_style(&self, index: usize) -> LineStyle {
        self.styles.get(index).copied().unwrap_or_default()
    }

    pub fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset;
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn set_gutter_enabled(&mut self, enabled: bool) {
        self.gutter_enabled = enabled;
        if !enabled {
            self.gutter_width = 0;
        }
    }

    pub fn gutter_enabled(&self) -> bool {
        self.gutter_enabled
    }

    pub fn gutter_width(&self) -> usize {
        self.gutter_width
    }
}

pub fn style_from_line(line: &LineChange) -> LineStyle {
    let heat = line
        .token_spans
        .iter()
        .map(|span| span.heat)
        .max_by_key(|heat| heat_rank(*heat))
        .unwrap_or_else(|| default_heat(line.kind));
    LineStyle {
        background_heat: heat,
        dimmed: line.kind == LineKind::Context,
    }
}

fn default_heat(kind: LineKind) -> HeatLevel {
    match kind {
        LineKind::Context => HeatLevel::Cool,
        LineKind::Added | LineKind::Removed => HeatLevel::Warm,
    }
}

fn heat_rank(heat: HeatLevel) -> u8 {
    match heat {
        HeatLevel::Cool => 0,
        HeatLevel::Warm => 1,
        HeatLevel::Hot => 2,
    }
}

pub fn line_background(style: LineStyle) -> Option<u32> {
    match style.background_heat {
        HeatLevel::Cool => None,
        HeatLevel::Warm => Some(if style.dimmed {
            LINE_WARM_DIMMED
        } else {
            LINE_WARM
        }),
        HeatLevel::Hot => Some(if style.dimmed {
            LINE_HOT_DIMMED
        } else {
            LINE_HOT
        }),
    }
}

pub fn token_background(heat: HeatLevel) -> Option<u32> {
    match heat {
        HeatLevel::Cool => None,
        HeatLevel::Warm => Some(TOKEN_WARM),
        HeatLevel::Hot => Some(TOKEN_HOT),
    }
}

pub fn token_kind_color(kind: TokenKind) -> Option<u32> {
    match kind {
        TokenKind::Plain => None,
        TokenKind::String => Some(0x9ecb8f),
        TokenKind::Comment => Some(0x7d8590),
        TokenKind::Keyword => Some(0xc792ea),
    }
}

pub fn text_color(dimmed: bool) -> u32 {
    if dimmed {
        TEXT_MUTED
    } else {
        TEXT_PRIMARY
    }
}

pub fn kind_glyph(kind: LineKind) -> &'static str {
    match kind {
        LineKind::Added => "+",
        LineKind::Removed => "-",
        LineKind::Context => " ",
    }
}

pub fn kind_color(kind: LineKind) -> u32 {
    match kind {
        LineKind::Added => TEXT_ADDED,
        LineKind::Removed => TEXT_REMOVED,
        LineKind::Context => GUTTER_TEXT,
    }
}

pub fn gutter_labels(
    old_line_no: Option<u32>,
    new_line_no: Option<u32>,
    width: usize,
) -> (String, String) {
    let old = old_line_no
        .map(|number| format!("{number:>width$}"))
        .unwrap_or_else(|| " ".repeat(width));
    let new = new_line_no
        .map(|number| format!("{number:>width$}"))
        .unwrap_or_else(|| " ".repeat(width));
    (old, new)
}

pub fn gutter_digit_width(lines: &[LineChange]) -> usize {
    let max_number = lines
        .iter()
        .flat_map(|line| [line.old_line_no, line.new_line_no])
        .flatten()
        .max()
        .unwrap_or(0);
    digits(max_number).max(1)
}

fn digits(number: u32) -> usize {
    if number == 0 {
        1
    } else {
        (number.ilog10() as usize) + 1
    }
}

#[cfg(test)]
mod tests {
    use qol_diff::{DiffStatus, FileDiff, Hunk, TokenSpan};

    use super::*;

    fn line(kind: LineKind, old: Option<u32>, new: Option<u32>) -> LineChange {
        LineChange {
            kind,
            text: String::new(),
            token_spans: Vec::new(),
            old_line_no: old,
            new_line_no: new,
        }
    }

    fn sample_diff(lines: Vec<LineChange>) -> FileDiff {
        FileDiff {
            old_path: "a.rs".to_string(),
            new_path: "a.rs".to_string(),
            status: DiffStatus::Modified,
            hunks: vec![Hunk {
                old_start: 1,
                old_lines: 0,
                new_start: 1,
                new_lines: 0,
                lines,
            }],
        }
    }

    #[test]
    fn gutter_labels_pad_right_and_blank_missing_sides() {
        assert_eq!(
            gutter_labels(Some(7), None, 4),
            ("   7".to_string(), "    ".to_string())
        );
        assert_eq!(
            gutter_labels(None, Some(42), 4),
            ("    ".to_string(), "  42".to_string())
        );
        assert_eq!(
            gutter_labels(Some(3), Some(300), 3),
            ("  3".to_string(), "300".to_string())
        );
        assert_eq!(
            gutter_labels(None, None, 2),
            ("  ".to_string(), "  ".to_string())
        );
    }

    #[test]
    fn gutter_width_tracks_the_widest_line_number() {
        let lines = vec![
            line(LineKind::Context, Some(9), Some(9)),
            line(LineKind::Added, None, Some(999)),
        ];
        assert_eq!(gutter_digit_width(&lines), 3);
        assert_eq!(gutter_digit_width(&[]), 1);
    }

    #[test]
    fn gutter_width_is_zero_when_disabled() {
        let mut surface = CodeSurface::new();
        surface.set_lines(&[line(LineKind::Context, Some(1234), Some(1234))]);
        assert_eq!(surface.gutter_width(), 4);
        surface.set_gutter_enabled(false);
        assert_eq!(surface.gutter_width(), 0);
        assert!(!surface.gutter_enabled());
        surface.set_gutter_enabled(true);
        surface.set_lines(&[line(LineKind::Context, Some(12), Some(12))]);
        assert_eq!(surface.gutter_width(), 2);
    }

    #[test]
    fn heat_backgrounds_are_stronger_from_warm_to_hot_and_for_tokens() {
        let line_warm = line_background(LineStyle {
            background_heat: HeatLevel::Warm,
            dimmed: false,
        })
        .unwrap();
        let line_hot = line_background(LineStyle {
            background_heat: HeatLevel::Hot,
            dimmed: false,
        })
        .unwrap();
        let token_warm = token_background(HeatLevel::Warm).unwrap();
        let token_hot = token_background(HeatLevel::Hot).unwrap();
        assert!(luminance(line_warm) < luminance(line_hot));
        assert!(luminance(line_hot) < luminance(token_warm));
        assert!(luminance(token_warm) < luminance(token_hot));
    }

    #[test]
    fn dimmed_variants_are_duller_than_their_heat_twins() {
        for heat in [HeatLevel::Warm, HeatLevel::Hot] {
            let plain = line_background(LineStyle {
                background_heat: heat,
                dimmed: false,
            })
            .unwrap();
            let dimmed = line_background(LineStyle {
                background_heat: heat,
                dimmed: true,
            })
            .unwrap();
            assert!(luminance(dimmed) < luminance(plain), "{heat:?}");
        }
    }

    #[test]
    fn cool_heat_renders_no_background() {
        assert_eq!(
            line_background(LineStyle {
                background_heat: HeatLevel::Cool,
                dimmed: true
            }),
            None
        );
        assert_eq!(token_background(HeatLevel::Cool), None);
    }

    #[test]
    fn dimmed_lines_use_muted_text() {
        assert_eq!(text_color(false), TEXT_PRIMARY);
        assert_eq!(text_color(true), TEXT_MUTED);
    }

    #[test]
    fn kind_glyphs_and_colors_distinguish_diff_sides() {
        assert_eq!(kind_glyph(LineKind::Added), "+");
        assert_eq!(kind_glyph(LineKind::Removed), "-");
        assert_eq!(kind_glyph(LineKind::Context), " ");
        assert_eq!(kind_color(LineKind::Added), TEXT_ADDED);
        assert_eq!(kind_color(LineKind::Removed), TEXT_REMOVED);
    }

    #[test]
    fn style_from_line_dimms_context_and_heats_changed_lines() {
        let context = line(LineKind::Context, Some(1), Some(1));
        assert_eq!(
            style_from_line(&context),
            LineStyle {
                background_heat: HeatLevel::Cool,
                dimmed: true
            }
        );
        let added = line(LineKind::Added, None, Some(2));
        assert_eq!(
            style_from_line(&added),
            LineStyle {
                background_heat: HeatLevel::Warm,
                dimmed: false
            }
        );
        let removed = line(LineKind::Removed, Some(2), None);
        assert_eq!(
            style_from_line(&removed),
            LineStyle {
                background_heat: HeatLevel::Warm,
                dimmed: false
            }
        );
    }

    #[test]
    fn token_spans_raise_the_line_heat() {
        let mut hot = line(LineKind::Context, Some(1), Some(1));
        hot.token_spans = vec![TokenSpan {
            start: 0,
            len: 2,
            heat: HeatLevel::Hot,
            kind: qol_diff::TokenKind::Plain,
        }];
        assert_eq!(
            style_from_line(&hot),
            LineStyle {
                background_heat: HeatLevel::Hot,
                dimmed: true
            }
        );
    }

    #[test]
    fn set_lines_derives_default_styles() {
        let mut surface = CodeSurface::new();
        surface.set_lines(&[
            line(LineKind::Context, Some(1), Some(1)),
            line(LineKind::Added, None, Some(2)),
        ]);
        assert_eq!(
            surface.line_style(0),
            style_from_line(&line(LineKind::Context, Some(1), Some(1)))
        );
        assert_eq!(
            surface.line_style(1),
            LineStyle {
                background_heat: HeatLevel::Warm,
                dimmed: false
            }
        );
    }

    #[test]
    fn line_style_defaults_and_out_of_range_are_harmless() {
        let surface = CodeSurface::new();
        assert_eq!(surface.line_style(0), LineStyle::default());
    }

    #[test]
    fn set_lines_resets_scroll() {
        let mut surface = CodeSurface::new();
        surface.set_lines(&vec![line(LineKind::Added, None, Some(1)); 10]);
        surface.set_scroll_offset(7);
        assert_eq!(surface.scroll_offset(), 7);
        surface.set_lines(&[]);
        assert_eq!(surface.scroll_offset(), 0);
    }

    #[test]
    fn scroll_offset_stores_what_it_is_given() {
        let mut surface = CodeSurface::new();
        surface.set_lines(&vec![line(LineKind::Added, None, Some(1)); 10]);
        surface.set_scroll_offset(usize::MAX);
        assert_eq!(
            surface.scroll_offset(),
            usize::MAX,
            "clamping lives in the view"
        );
        surface.set_scroll_offset(3);
        assert_eq!(surface.scroll_offset(), 3);
        surface.set_scroll_offset(0);
        assert_eq!(surface.scroll_offset(), 0);
    }

    fn luminance(hex: u32) -> f32 {
        let red = ((hex >> 16) & 0xff) as f32 / 255.0;
        let green = ((hex >> 8) & 0xff) as f32 / 255.0;
        let blue = (hex & 0xff) as f32 / 255.0;
        0.2126 * red + 0.7152 * green + 0.0722 * blue
    }

    #[test]
    fn sample_diff_is_renderable() {
        let diff = sample_diff(vec![line(LineKind::Context, Some(1), Some(1))]);
        assert_eq!(diff.status, DiffStatus::Modified);
    }
}
