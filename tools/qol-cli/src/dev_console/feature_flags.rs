use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use super::picker::{
    filter_text, filter_text_width, picker_brick_layout, PickerBrick, FILTER_BRICK_CHROME,
};
use super::render_util::{accent, panel_width, render_bottom_panel};
use super::Dash;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum FeatureFlag {}

pub(super) struct FeatureFlagDef {
    pub(super) flag: FeatureFlag,
    pub(super) id: &'static str,
    pub(super) label: &'static str,
}

pub(super) const FEATURE_FLAGS: &[FeatureFlagDef] = &[];

#[derive(Debug, PartialEq, Eq, Default)]
pub(super) struct FeatureFlags {
    enabled: Vec<FeatureFlag>,
}

impl FeatureFlags {
    pub(super) fn enabled(&self, flag: FeatureFlag) -> bool {
        self.enabled.contains(&flag)
    }

    pub(super) fn ids(&self) -> Vec<String> {
        FEATURE_FLAGS
            .iter()
            .filter(|def| self.enabled(def.flag))
            .map(|def| def.id.to_string())
            .collect()
    }

    pub(super) fn from_ids(ids: &[String]) -> Self {
        let enabled = FEATURE_FLAGS
            .iter()
            .filter(|def| ids.iter().any(|id| id == def.id))
            .map(|def| def.flag)
            .collect();
        Self { enabled }
    }
}

#[derive(Debug, PartialEq, Eq, Default)]
pub(super) struct FeatureFlagPanel {
    pub(super) open: bool,
    pub(super) selected: usize,
    pub(super) layout_width: usize,
}

impl FeatureFlagPanel {
    pub(super) fn is_active(&self) -> bool {
        self.open
    }
}

pub(super) fn toggle_feature_flag(flag: FeatureFlag) {
    match flag {}
}

pub(super) fn draw_feature_flags_panel(
    frame: &mut Frame,
    dash: &mut Dash,
    area: Rect,
    accent: Color,
) {
    if !dash.feature_panel.is_active() {
        return;
    }
    dash.feature_panel.layout_width = panel_width(area).saturating_sub(2) as usize;
    let rows = feature_flag_panel_rows(dash);
    render_bottom_panel(frame, area, "feature flags", rows, accent);
}

fn feature_flag_panel_rows(dash: &Dash) -> Vec<Line<'static>> {
    if FEATURE_FLAGS.is_empty() {
        return vec![Line::from(" no feature flags".fg(Color::DarkGray))];
    }
    let mut rows = feature_flag_brick_rows(dash);
    if let Some(def) = FEATURE_FLAGS.get(dash.feature_panel.selected) {
        let state = if dash.features.enabled(def.flag) {
            "on"
        } else {
            "off"
        };
        rows.push(Line::from(""));
        rows.push(Line::from(vec![
            format!(" {state:<3} ")
                .fg(feature_flag_color(dash, def))
                .bold(),
            def.label.fg(Color::White),
        ]));
    }
    rows
}

fn feature_flag_brick_rows(dash: &Dash) -> Vec<Line<'static>> {
    let layout = feature_flag_brick_layout(dash.feature_panel.layout_width);
    let Some(max_row) = layout.iter().map(|brick| brick.row).max() else {
        return Vec::new();
    };
    (0..=max_row)
        .map(|row| feature_flag_brick_row(dash, &layout, row))
        .collect()
}

fn feature_flag_brick_row(dash: &Dash, layout: &[PickerBrick], row: usize) -> Line<'static> {
    let mut spans = Vec::new();
    let mut x = 0;
    for brick in layout.iter().filter(|brick| brick.row == row) {
        if brick.x > x {
            spans.push(Span::raw(" ".repeat(brick.x - x)));
        }
        let def = &FEATURE_FLAGS[brick.index];
        spans.extend(feature_flag_brick_spans(
            dash,
            def,
            brick.index == dash.feature_panel.selected,
            brick.width,
        ));
        x = brick.x + brick.width;
    }
    Line::from(spans)
}

pub(super) fn feature_flag_brick_layout(width: usize) -> Vec<PickerBrick> {
    picker_brick_layout(FEATURE_FLAGS, width, feature_flag_brick_width)
}

fn feature_flag_brick_width(flag: &FeatureFlagDef, row_width: usize) -> usize {
    let max_text_width = filter_text_width(row_width);
    FILTER_BRICK_CHROME + filter_text(flag.id, max_text_width).chars().count()
}

fn feature_flag_brick_spans(
    dash: &Dash,
    flag: &FeatureFlagDef,
    selected: bool,
    width: usize,
) -> Vec<Span<'static>> {
    let text = filter_text(flag.id, filter_text_width(width));
    let enabled = dash.features.enabled(flag.flag);
    let text_style = if selected {
        Style::new().fg(Color::White).bg(Color::Rgb(38, 44, 74))
    } else if enabled {
        Style::new().fg(Color::White)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let symbol_style = if selected {
        Style::new()
            .fg(feature_flag_color(dash, flag))
            .bg(Color::Rgb(38, 44, 74))
            .bold()
    } else {
        Style::new().fg(feature_flag_color(dash, flag)).bold()
    };
    let edge = if selected { ("[", "]") } else { (" ", " ") };
    let symbol = if enabled { "*" } else { "." };
    vec![
        Span::styled(edge.0.to_string(), text_style),
        Span::styled(symbol.to_string(), symbol_style),
        Span::styled(" ".to_string(), text_style),
        Span::styled(text, text_style),
        Span::styled(edge.1.to_string(), text_style),
    ]
}

fn feature_flag_color(dash: &Dash, flag: &FeatureFlagDef) -> Color {
    if dash.features.enabled(flag.flag) {
        accent()
    } else {
        Color::DarkGray
    }
}
