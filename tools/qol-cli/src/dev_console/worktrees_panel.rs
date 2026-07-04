use std::collections::BTreeSet;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use super::picker::{
    filter_text, filter_text_width, move_picker_selection, picker_brick_layout, PickerBrick,
    PickerMove, FILTER_BRICK_CHROME,
};
use super::render_util::{accent, panel_width, render_bottom_panel};
use super::{Dash, WorktreeSelection, ORANGE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorktreeTarget {
    pub(super) branch: Option<String>,
    pub(super) id: String,
}

#[derive(Debug, PartialEq, Eq, Default)]
pub(super) struct WorktreePanel {
    pub(super) open: bool,
    pub(super) selected: usize,
    pub(super) layout_width: usize,
    pub(super) targets: Vec<WorktreeTarget>,
}

impl WorktreePanel {
    pub(super) fn is_active(&self) -> bool {
        self.open
    }
}

pub(super) fn open_worktrees_panel(dash: &mut Dash) {
    dash.worktree_panel.targets = worktree_targets(&dash.base_label);
    dash.worktree_panel.selected = dash
        .worktree_panel
        .targets
        .iter()
        .position(|target| target.branch.as_deref() == dash.effective_worktree_target())
        .unwrap_or(0);
    dash.worktree_panel.open = true;
}

pub(super) fn move_worktree_selection(dash: &mut Dash, direction: PickerMove) {
    let layout = worktree_brick_layout(
        &dash.worktree_panel.targets,
        dash.worktree_panel.layout_width,
    );
    move_picker_selection(
        &mut dash.worktree_panel.selected,
        dash.worktree_panel.targets.len(),
        direction,
        &layout,
    );
}

pub(super) fn arm_selected_worktree(dash: &mut Dash) {
    let Some(target) = dash
        .worktree_panel
        .targets
        .get(dash.worktree_panel.selected)
        .cloned()
    else {
        return;
    };
    dash.worktree_selection = WorktreeSelection::Pin(target.branch);
    dash.armed = true;
    dash.worktree_panel.open = false;
}

pub(super) fn draw_worktrees_panel(frame: &mut Frame, dash: &mut Dash, area: Rect, accent: Color) {
    if !dash.worktree_panel.is_active() {
        return;
    }
    dash.worktree_panel.layout_width = panel_width(area).saturating_sub(2) as usize;
    let rows = worktree_panel_rows(dash);
    render_bottom_panel(frame, area, "worktree", rows, accent);
}

pub(super) fn worktree_panel_rows(dash: &Dash) -> Vec<Line<'static>> {
    let mut rows = worktree_brick_rows(dash);
    if dash.worktree_panel.targets.len() <= 1 {
        rows.push(Line::from(""));
        rows.push(Line::from(" no worktrees".fg(Color::DarkGray)));
    }
    if let Some(target) = dash
        .worktree_panel
        .targets
        .get(dash.worktree_panel.selected)
    {
        rows.push(Line::from(""));
        rows.push(Line::from(vec![
            format!(" {:<7} ", target_state(dash, target))
                .fg(worktree_target_color(dash, target))
                .bold(),
            target.id.clone().fg(Color::White),
        ]));
    }
    rows
}

fn worktree_brick_rows(dash: &Dash) -> Vec<Line<'static>> {
    let layout = worktree_brick_layout(
        &dash.worktree_panel.targets,
        dash.worktree_panel.layout_width,
    );
    let Some(max_row) = layout.iter().map(|brick| brick.row).max() else {
        return Vec::new();
    };
    (0..=max_row)
        .map(|row| worktree_brick_row(dash, &layout, row))
        .collect()
}

fn worktree_brick_row(dash: &Dash, layout: &[PickerBrick], row: usize) -> Line<'static> {
    let mut spans = Vec::new();
    let mut x = 0;
    for brick in layout.iter().filter(|brick| brick.row == row) {
        if brick.x > x {
            spans.push(Span::raw(" ".repeat(brick.x - x)));
        }
        let target = &dash.worktree_panel.targets[brick.index];
        spans.extend(worktree_brick_spans(
            dash,
            target,
            brick.index == dash.worktree_panel.selected,
            brick.width,
        ));
        x = brick.x + brick.width;
    }
    Line::from(spans)
}

pub(super) fn worktree_brick_layout(targets: &[WorktreeTarget], width: usize) -> Vec<PickerBrick> {
    picker_brick_layout(targets, width, worktree_brick_width)
}

fn worktree_brick_width(target: &WorktreeTarget, row_width: usize) -> usize {
    let max_text_width = filter_text_width(row_width);
    FILTER_BRICK_CHROME + filter_text(&target.id, max_text_width).chars().count()
}

fn worktree_brick_spans(
    dash: &Dash,
    target: &WorktreeTarget,
    selected: bool,
    width: usize,
) -> Vec<Span<'static>> {
    let text = filter_text(&target.id, filter_text_width(width));
    let color = worktree_target_color(dash, target);
    let text_style = if selected {
        Style::new().fg(Color::White).bg(Color::Rgb(38, 44, 74))
    } else if is_effective_target(dash, target) {
        Style::new().fg(Color::White)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let symbol_style = if selected {
        Style::new().fg(color).bg(Color::Rgb(38, 44, 74)).bold()
    } else {
        Style::new().fg(color).bold()
    };
    let edge = if selected { ("[", "]") } else { (" ", " ") };
    let symbol = if is_effective_target(dash, target) {
        "*"
    } else if target.branch == dash.running_branch {
        "o"
    } else {
        "."
    };
    vec![
        Span::styled(edge.0.to_string(), text_style),
        Span::styled(symbol.to_string(), symbol_style),
        Span::styled(" ".to_string(), text_style),
        Span::styled(text, text_style),
        Span::styled(edge.1.to_string(), text_style),
    ]
}

fn worktree_targets(base_label: &str) -> Vec<WorktreeTarget> {
    let mut targets = vec![WorktreeTarget {
        branch: None,
        id: base_label.to_string(),
    }];
    let Ok(root) = crate::workspace::repo_root() else {
        return targets;
    };
    let branches: BTreeSet<String> = qol_dev_build::tray::list_worktrees(&root)
        .into_iter()
        .map(|worktree| worktree.branch)
        .collect();
    targets.extend(branches.into_iter().map(|branch| WorktreeTarget {
        id: branch.clone(),
        branch: Some(branch),
    }));
    targets
}

pub(super) fn target_label(branch: Option<&str>, base_label: &str) -> String {
    branch.unwrap_or(base_label).to_string()
}

fn is_effective_target(dash: &Dash, target: &WorktreeTarget) -> bool {
    target.branch.as_deref() == dash.effective_worktree_target()
}

fn target_state(dash: &Dash, target: &WorktreeTarget) -> &'static str {
    if target.branch == dash.running_branch {
        return "running";
    }
    if is_effective_target(dash, target) {
        return "armed";
    }
    "target"
}

fn worktree_target_color(dash: &Dash, target: &WorktreeTarget) -> Color {
    if is_effective_target(dash, target) && target.branch != dash.running_branch {
        return ORANGE;
    }
    if target.branch == dash.running_branch {
        return accent();
    }
    Color::DarkGray
}
