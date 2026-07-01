use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Clear;
use ratatui::Frame;
use serde::{Deserialize, Serialize};

use super::picker::{
    filter_text, filter_text_width, picker_brick_layout, PickerBrick, FILTER_BRICK_CHROME,
    FILTER_PANEL_MAX_WIDTH, FILTER_PANEL_MIN_WIDTH,
};
use super::render_util::SignBox;
use super::{Dash, View};

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum FilterStrategy {
    Include,
    Exclude,
}

impl FilterStrategy {
    pub(super) fn symbol(self) -> &'static str {
        match self {
            Self::Include => "+",
            Self::Exclude => "-",
        }
    }

    pub(super) fn color(self) -> Color {
        match self {
            Self::Include => Color::Green,
            Self::Exclude => Color::Red,
        }
    }

    pub(super) fn cycle(self) -> Self {
        match self {
            Self::Include => Self::Exclude,
            Self::Exclude => Self::Include,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(super) struct LogFilter {
    pub(super) strategy: FilterStrategy,
    pub(super) text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FilterScope {
    Logs,
    Trace,
    Emu,
}

pub(super) fn filter_scope(view: View) -> Option<FilterScope> {
    match view {
        View::Logs => Some(FilterScope::Logs),
        View::Trace => Some(FilterScope::Trace),
        View::EmuDetail => Some(FilterScope::Emu),
        View::Dashboard | View::Doctor | View::Plugins | View::Emu | View::Endpoints => None,
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ViewFilters {
    #[serde(default)]
    pub(super) logs: Vec<LogFilter>,
    #[serde(default)]
    pub(super) trace: Vec<LogFilter>,
    #[serde(default)]
    pub(super) emu: Vec<LogFilter>,
}

impl ViewFilters {
    pub(super) fn for_view(&self, view: View) -> &[LogFilter] {
        match filter_scope(view) {
            Some(FilterScope::Logs) => &self.logs,
            Some(FilterScope::Trace) => &self.trace,
            Some(FilterScope::Emu) => &self.emu,
            None => &[],
        }
    }

    pub(super) fn for_view_mut(&mut self, view: View) -> Option<&mut Vec<LogFilter>> {
        match filter_scope(view) {
            Some(FilterScope::Logs) => Some(&mut self.logs),
            Some(FilterScope::Trace) => Some(&mut self.trace),
            Some(FilterScope::Emu) => Some(&mut self.emu),
            None => None,
        }
    }
}

pub(super) fn line_matches_filters(line: &str, filters: &[LogFilter]) -> bool {
    let mut has_include = false;
    let mut included = false;
    for filter in filters {
        if filter.text.is_empty() {
            continue;
        }
        let matched = line.contains(&filter.text);
        match filter.strategy {
            FilterStrategy::Exclude if matched => return false,
            FilterStrategy::Include => {
                has_include = true;
                included |= matched;
            }
            FilterStrategy::Exclude => {}
        }
    }
    !has_include || included
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum FilterState {
    Closed,
    Managing,
    Editing {
        index: Option<usize>,
        draft: String,
        strategy: FilterStrategy,
    },
}

impl FilterState {
    pub(super) fn is_active(&self) -> bool {
        !matches!(self, Self::Closed)
    }
}

pub(super) fn draw_filter_panel(frame: &mut Frame, dash: &mut Dash, area: Rect, accent: Color) {
    if !dash.filter_state.is_active() {
        return;
    }
    let width = if area.width <= FILTER_PANEL_MIN_WIDTH {
        area.width
    } else {
        area.width
            .saturating_sub(4)
            .clamp(FILTER_PANEL_MIN_WIDTH, FILTER_PANEL_MAX_WIDTH)
    };
    dash.filter_layout_width = width.saturating_sub(2) as usize;
    let mut rows = filter_panel_rows(dash);
    let height = (rows.len() as u16 + SignBox::CHROME_ROWS).min(area.height);
    if width == 0 || height == 0 {
        return;
    }
    rows.truncate(SignBox::capacity(height));
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + area.height.saturating_sub(height + 1),
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    SignBox {
        title: "filters",
        rows,
    }
    .render(frame, rect, accent);
}

pub(super) fn filter_panel_rows(dash: &Dash) -> Vec<Line<'static>> {
    match &dash.filter_state {
        FilterState::Closed => Vec::new(),
        FilterState::Managing if dash.active_filters().is_empty() => vec![
            Line::from(" no filters".fg(Color::DarkGray)),
            Line::from(" enter add".fg(Color::DarkGray)),
        ],
        FilterState::Managing => filter_brick_rows(
            dash.active_filters(),
            dash.filter_index,
            dash.filter_layout_width,
        ),
        FilterState::Editing {
            index,
            draft,
            strategy,
        } => {
            let label = if index.is_some() { " edit" } else { " add" };
            vec![Line::from(vec![
                label.fg(Color::DarkGray),
                " ".into(),
                strategy.symbol().fg(strategy.color()).bold(),
                " ".into(),
                format!("{draft}_").fg(Color::White),
            ])]
        }
    }
}

pub(super) fn filter_brick_rows(
    filters: &[LogFilter],
    selected_index: usize,
    width: usize,
) -> Vec<Line<'static>> {
    let layout = filter_brick_layout(filters, width);
    let Some(max_row) = layout.iter().map(|brick| brick.row).max() else {
        return Vec::new();
    };
    (0..=max_row)
        .map(|row| filter_brick_row(filters, selected_index, &layout, row))
        .collect()
}

fn filter_brick_row(
    filters: &[LogFilter],
    selected_index: usize,
    layout: &[PickerBrick],
    row: usize,
) -> Line<'static> {
    let mut spans = Vec::new();
    let mut x = 0;
    for brick in layout.iter().filter(|brick| brick.row == row) {
        if brick.x > x {
            spans.push(Span::raw(" ".repeat(brick.x - x)));
        }
        let filter = &filters[brick.index];
        spans.extend(filter_brick_spans(
            filter,
            brick.index == selected_index,
            brick.width,
        ));
        x = brick.x + brick.width;
    }
    Line::from(spans)
}

pub(super) fn filter_brick_layout(filters: &[LogFilter], width: usize) -> Vec<PickerBrick> {
    picker_brick_layout(filters, width, filter_brick_width)
}

fn filter_brick_width(filter: &LogFilter, row_width: usize) -> usize {
    let max_text_width = filter_text_width(row_width);
    FILTER_BRICK_CHROME + filter_text(&filter.text, max_text_width).chars().count()
}

fn filter_brick_spans(filter: &LogFilter, selected: bool, width: usize) -> Vec<Span<'static>> {
    let text = filter_text(&filter.text, filter_text_width(width));
    let text_style = if selected {
        Style::new().fg(Color::White).bg(Color::Rgb(38, 44, 74))
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let symbol_style = if selected {
        Style::new()
            .fg(filter.strategy.color())
            .bg(Color::Rgb(38, 44, 74))
            .bold()
    } else {
        Style::new().fg(filter.strategy.color()).bold()
    };
    let edge = if selected { ("[", "]") } else { (" ", " ") };
    vec![
        Span::styled(edge.0.to_string(), text_style),
        Span::styled(filter.strategy.symbol().to_string(), symbol_style),
        Span::styled(" ".to_string(), text_style),
        Span::styled(text, text_style),
        Span::styled(edge.1.to_string(), text_style),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_filter(strategy: FilterStrategy, text: &str) -> LogFilter {
        LogFilter {
            strategy,
            text: text.to_string(),
        }
    }

    #[test]
    fn line_matches_filters_combines_include_and_exclude_rules() {
        let filters = vec![
            log_filter(FilterStrategy::Include, "shortcut"),
            log_filter(FilterStrategy::Include, "trace"),
            log_filter(FilterStrategy::Exclude, "success"),
        ];
        assert!(line_matches_filters("shortcut failed", &filters));
        assert!(line_matches_filters("trace emitted", &filters));
        assert!(!line_matches_filters("profile synced", &filters));
        assert!(!line_matches_filters("shortcut success", &filters));
    }

    #[test]
    fn exclude_only_filters_keep_non_matching_lines() {
        let filters = vec![log_filter(FilterStrategy::Exclude, "noise")];
        assert!(line_matches_filters("important trace", &filters));
        assert!(!line_matches_filters("noise trace", &filters));
    }
}
