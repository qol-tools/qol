use std::time::Duration;

use ratatui::layout::Rect;
use ratatui::style::{Color, Stylize};
use ratatui::text::Line;
use ratatui::Frame;

use super::render_util::{accent, format_duration, render_compact_bottom_panel};
use super::Dash;

pub(super) struct Activity {
    pub(super) title: &'static str,
    pub(super) phase: String,
    pub(super) detail: String,
    pub(super) elapsed: Duration,
}

pub(super) fn activity_rows(activity: &Activity) -> Vec<Line<'static>> {
    let mut spans = vec![
        " ● ".fg(accent()).bold(),
        activity.phase.clone().fg(Color::White).bold(),
    ];
    if !activity.detail.is_empty() {
        spans.push(" · ".fg(Color::DarkGray));
        spans.push(activity.detail.clone().fg(Color::Gray));
    }
    spans.push(format!(" · {}", format_duration(activity.elapsed)).fg(Color::DarkGray));
    spans.push(" ".into());
    vec![Line::from(spans)]
}

pub(super) fn draw_activity(frame: &mut Frame, dash: &Dash, area: Rect, accent: Color) {
    if dash.quit_prompt_active() {
        return;
    }
    let Some(activity) = dash.activity() else {
        return;
    };
    render_compact_bottom_panel(
        frame,
        area,
        activity.title,
        activity_rows(&activity),
        accent,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(activity: &Activity) -> String {
        activity_rows(activity)[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn activity_row_drops_the_separator_when_no_detail_is_known_yet() {
        let cases = [
            ("build", "qol-tray dev", " ● build · qol-tray dev · 0m12s "),
            ("fixing", "", " ● fixing · 0m12s "),
        ];
        for (phase, detail, expected) in cases {
            let activity = Activity {
                title: "reload",
                phase: phase.to_string(),
                detail: detail.to_string(),
                elapsed: Duration::from_secs(12),
            };
            assert_eq!(row_text(&activity), expected, "phase: {phase}");
        }
    }
}
