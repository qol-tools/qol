use gpui::*;

use super::layout::{HEADER_HEIGHT, WINDOW_WIDTH};
use super::trace;
use super::LauncherView;
use crate::discovery::search::{ResultSource, SearchMode};

const HELP_ROW_HEIGHT: f32 = 17.0;
type HelpRows = &'static [(&'static str, &'static str)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuKind {
    Help,
    Options,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OptionAction {
    Apps,
    Files,
    Open,
    OpenFolder,
    BoostUp,
    BoostDown,
}

impl OptionAction {
    fn label(self) -> &'static str {
        match self {
            Self::Apps => "Apps",
            Self::Files => "Files",
            Self::Open => "Open",
            Self::OpenFolder => "Open containing folder",
            Self::BoostUp => "Boost result",
            Self::BoostDown => "Reduce boost",
        }
    }

    fn shortcut(self) -> &'static str {
        match self {
            Self::Open => "Enter",
            Self::OpenFolder => "Shift+Enter",
            Self::BoostUp => "Ctrl+→ / Alt+→",
            Self::BoostDown => "Ctrl+← / Alt+←",
            Self::Apps | Self::Files => "",
        }
    }
}

fn option_actions(source: Option<ResultSource>, boost: i32) -> Vec<OptionAction> {
    let mut actions = vec![OptionAction::Apps, OptionAction::Files];
    if source.is_some() {
        actions.push(OptionAction::Open);
    }
    if matches!(source, Some(ResultSource::App | ResultSource::File)) {
        actions.push(OptionAction::OpenFolder);
    }
    if matches!(source, Some(ResultSource::App)) {
        actions.push(OptionAction::BoostUp);
        if boost > 0 {
            actions.push(OptionAction::BoostDown);
        }
    }
    actions
}

const HELP_SEARCH_LEFT_ROWS: HelpRows = &[
    ("Search", ""),
    ("Move through results", "↑ / ↓"),
    ("Switch Apps / Files", "Tab / Shift+Tab"),
    ("Narrow matches", "Ctrl+↑"),
    ("Broaden matches", "Ctrl+↓"),
    ("Selected result", ""),
    ("Open", "Enter"),
    ("Apps and files", ""),
    ("Open containing folder", "Shift+Enter"),
    ("Ranking · Apps", ""),
    ("Raise rank", "Ctrl+→ / Alt+→"),
    ("Lower rank", "Ctrl+← / Alt+←"),
];

const HELP_QUERY_ROWS: HelpRows = &[
    ("Query", ""),
    ("Move caret", "← / →"),
    ("Select text", "Shift+← / →"),
    ("Start / end", "Home / End"),
    ("Delete text", "Backspace / Del"),
    ("Select all", "Ctrl+A"),
    ("Copy / cut / paste", "Ctrl+C/X/V"),
];

const HELP_SEARCH_WINDOW_ROWS: HelpRows = &[
    ("Window", ""),
    ("Options", "Alt+Enter"),
    ("Help", "Alt+H"),
    ("Dismiss", "Esc"),
];

const HELP_FLOW_LEFT_ROWS: HelpRows = &[
    ("Flow results", ""),
    ("Move through results", "↑ / ↓"),
    ("Open detail", "Enter"),
    ("Dislike result", "Alt+X"),
    ("Back to search", "Esc"),
];

const HELP_DETAIL_LEFT_ROWS: HelpRows = &[
    ("Flow detail", ""),
    ("Scroll", "↑ / ↓"),
    ("Activate", "Enter"),
    ("Back to results", "Esc"),
];

const HELP_FLOW_WINDOW_ROWS: HelpRows = &[("Window", ""), ("Help", "Alt+H")];

fn help_height(left: HelpRows, right: HelpRows, right_extra: HelpRows) -> f32 {
    qol_gpui::theme::HEIGHT_INLINE
        + left.len().max(right.len() + right_extra.len()) as f32 * HELP_ROW_HEIGHT
        + 2.0 * qol_gpui::theme::SPACE_TIGHT
        + 10.0
}

impl LauncherView {
    pub(super) fn toggle_menu(&mut self, kind: MenuKind, cx: &mut Context<Self>) {
        if kind == MenuKind::Options && self.state.flow.is_some() {
            return;
        }
        self.menu_kind = if self.menu_kind == Some(kind) {
            None
        } else {
            Some(kind)
        };
        self.menu_selected = 0;
        self.menu_scroll = ScrollHandle::new();
        trace::menu(
            match kind {
                MenuKind::Help => "help",
                MenuKind::Options => "options",
            },
            if self.menu_kind.is_some() {
                "open"
            } else {
                "close"
            },
        );
        cx.notify();
    }

    pub(super) fn set_search_mode(&mut self, mode: SearchMode, cx: &mut Context<Self>) {
        self.menu_kind = None;
        self.state.mode = mode;
        self.state.clear_launch_error();
        self.state.reset_results_position();
        self.dispatch_query_change(cx);
    }

    pub(super) fn menu_height(&self) -> f32 {
        match self.menu_kind {
            Some(MenuKind::Help) => {
                let (left, right, right_extra) = self.help_rows();
                help_height(left, right, right_extra)
            }
            Some(MenuKind::Options) => {
                let selected_section = usize::from(self.store.result_count() > 0);
                let rows = 2 + self.available_options().len() + selected_section;
                rows as f32 * qol_gpui::theme::HEIGHT_INLINE
                    + 2.0 * qol_gpui::theme::SPACE_TIGHT
                    + 2.0
            }
            None => 0.0,
        }
    }

    fn help_rows(&self) -> (HelpRows, HelpRows, HelpRows) {
        if self.state.flow_detail_open() {
            return (HELP_DETAIL_LEFT_ROWS, &[], HELP_FLOW_WINDOW_ROWS);
        }
        if self.state.flow.is_some() {
            return (HELP_FLOW_LEFT_ROWS, HELP_QUERY_ROWS, HELP_FLOW_WINDOW_ROWS);
        }
        (
            HELP_SEARCH_LEFT_ROWS,
            HELP_QUERY_ROWS,
            HELP_SEARCH_WINDOW_ROWS,
        )
    }

    pub(super) fn handle_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let modifiers = &event.keystroke.modifiers;
        if modifiers.alt && key == "h" {
            self.toggle_menu(MenuKind::Help, cx);
            return true;
        }
        if modifiers.alt && key == "enter" && self.state.flow.is_none() {
            self.toggle_menu(MenuKind::Options, cx);
            return true;
        }
        let Some(kind) = self.menu_kind else {
            return false;
        };
        if matches!(key, "escape" | "esc") {
            self.menu_kind = None;
            trace::menu("any", "escape");
            cx.notify();
            return true;
        }
        if key == "tab" {
            self.menu_kind = None;
            return false;
        }
        if kind == MenuKind::Help {
            self.menu_kind = None;
            return false;
        }
        if matches!(key, "up" | "down") {
            let len = self.available_options().len();
            self.menu_selected = if key == "down" {
                (self.menu_selected + 1) % len
            } else {
                (self.menu_selected + len - 1) % len
            };
            cx.notify();
            return true;
        }
        if key == "enter" {
            if let Some(action) = self.available_options().get(self.menu_selected).copied() {
                self.activate_option(action, window, cx);
            }
            return true;
        }
        self.menu_kind = None;
        false
    }

    fn available_options(&self) -> Vec<OptionAction> {
        let selected = self.store.get(self.state.scroll_list.selected);
        option_actions(
            selected.map(|scored| scored.source),
            selected.map_or(0, |scored| scored.manual_boost),
        )
    }

    fn activate_option(
        &mut self,
        action: OptionAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu_kind = None;
        trace::menu("options", action.label());
        match action {
            OptionAction::Apps => self.set_search_mode(SearchMode::Apps, cx),
            OptionAction::Files => self.set_search_mode(SearchMode::Files, cx),
            OptionAction::Open => self.launch_selected(window, cx),
            OptionAction::OpenFolder => self.open_selected_folder(window, cx),
            OptionAction::BoostUp => self.adjust_selected_boost(25, cx),
            OptionAction::BoostDown => self.adjust_selected_boost(-25, cx),
        }
        cx.notify();
    }

    pub(super) fn menu_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let kit = qol_gpui::kit::kit();
        let kind = self.menu_kind.expect("menu overlay requires an open menu");
        let width = if kind == MenuKind::Help {
            WINDOW_WIDTH - 16.0
        } else {
            280.0
        };
        let mut panel = div()
            .id("launcher-menu")
            .absolute()
            .top(px(HEADER_HEIGHT + 4.0))
            .right(px(8.0))
            .w(px(width))
            .h(px(self.menu_height()))
            .p(px(qol_gpui::theme::SPACE_TIGHT))
            .rounded(px(qol_gpui::theme::RADIUS_CONTROL))
            .border(px(1.0))
            .border_color(rgba(kit.washes.hairline_strong.packed()))
            .bg(rgb(kit.palette.surface_raised))
            .shadow(qol_gpui::kit::float_shadow(kit.palette.text_primary))
            .track_scroll(&self.menu_scroll)
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if kind == MenuKind::Help {
            let (left, right, right_extra) = self.help_rows();
            panel = panel.child(menu_title("Help", "Alt+H"));
            panel = panel.child(
                div()
                    .flex()
                    .gap(px(qol_gpui::theme::SPACE_INSET))
                    .child(help_column(left, &[]))
                    .child(help_column(right, right_extra)),
            );
            return panel;
        }
        panel = panel.child(menu_title("Options", "Alt+Enter"));
        panel = panel.child(menu_section("Search type"));
        let selected = self.store.get(self.state.scroll_list.selected);
        let actions = self.available_options();
        for (index, action) in actions.into_iter().enumerate() {
            if index == 2 {
                if let Some(scored) = selected {
                    panel = panel.child(menu_section(self.store.name(scored)));
                }
            }
            panel = panel.child(option_row(
                action,
                index == self.menu_selected,
                self.state.mode,
                cx,
            ));
        }
        panel
    }
}

fn menu_title(label: &str, shortcut: &str) -> Div {
    let kit = qol_gpui::kit::kit();
    div()
        .flex_none()
        .h(px(qol_gpui::theme::HEIGHT_INLINE))
        .px(px(qol_gpui::theme::SPACE_INSET))
        .flex()
        .items_center()
        .justify_between()
        .text_color(rgb(kit.palette.text_muted))
        .text_size(px(qol_gpui::theme::TEXT_NANO))
        .child(label.to_owned())
        .child(shortcut.to_owned())
}

fn menu_section(label: &str) -> Div {
    let kit = qol_gpui::kit::kit();
    div()
        .flex_none()
        .h(px(qol_gpui::theme::HEIGHT_INLINE))
        .px(px(qol_gpui::theme::SPACE_INSET))
        .flex()
        .items_center()
        .text_color(rgb(kit.palette.text_muted))
        .text_size(px(qol_gpui::theme::TEXT_NANO))
        .child(label.to_owned())
}

fn help_row(label: &str, shortcut: &str) -> Div {
    let kit = qol_gpui::kit::kit();
    div()
        .flex_none()
        .h(px(HELP_ROW_HEIGHT))
        .px(px(qol_gpui::theme::SPACE_INSET))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(qol_gpui::theme::SPACE_SNUG))
        .text_color(rgb(kit.palette.text_primary))
        .text_size(px(qol_gpui::theme::TEXT_NANO))
        .child(div().flex_1().min_w_0().truncate().child(label.to_owned()))
        .child(
            div()
                .flex_none()
                .font_family(SharedString::from(qol_gpui::theme::font_mono()))
                .text_color(rgb(kit.palette.text_muted))
                .text_size(px(qol_gpui::theme::TEXT_IDENTITY))
                .child(display_shortcut(shortcut)),
        )
}

fn help_column(rows: HelpRows, extra: HelpRows) -> Div {
    let kit = qol_gpui::kit::kit();
    let mut column = div().flex_1().min_w_0().flex().flex_col();
    for (label, shortcut) in rows.iter().chain(extra.iter()) {
        column = if shortcut.is_empty() {
            column.child(
                div()
                    .flex_none()
                    .h(px(HELP_ROW_HEIGHT))
                    .px(px(qol_gpui::theme::SPACE_INSET))
                    .flex()
                    .items_center()
                    .text_color(rgb(kit.palette.text_muted))
                    .text_size(px(qol_gpui::theme::TEXT_NANO))
                    .child((*label).to_owned()),
            )
        } else {
            column.child(help_row(label, shortcut))
        };
    }
    column
}

fn display_shortcut_for(shortcut: &str, macos: bool) -> String {
    if macos {
        return shortcut.replace("Ctrl+", "Cmd+");
    }
    shortcut.to_owned()
}

fn display_shortcut(shortcut: &str) -> String {
    display_shortcut_for(shortcut, cfg!(target_os = "macos"))
}

fn option_row(
    action: OptionAction,
    selected: bool,
    mode: SearchMode,
    cx: &mut Context<LauncherView>,
) -> impl IntoElement {
    let kit = qol_gpui::kit::kit();
    let mark = match action {
        OptionAction::Apps if mode == SearchMode::Apps => "✓",
        OptionAction::Files if mode == SearchMode::Files => "✓",
        _ => action.shortcut(),
    };
    let row = div()
        .id(SharedString::from(format!("launcher-option-{action:?}")))
        .h(px(qol_gpui::theme::HEIGHT_INLINE))
        .px(px(qol_gpui::theme::SPACE_INSET))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(qol_gpui::theme::SPACE_SNUG))
        .rounded(px(qol_gpui::theme::RADIUS_TIGHT))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
        .text_color(rgb(kit.palette.text_primary))
        .text_size(px(qol_gpui::theme::TEXT_MICRO))
        .child(div().flex_1().min_w_0().truncate().child(action.label()))
        .child(
            div()
                .flex_none()
                .font_family(SharedString::from(qol_gpui::theme::font_mono()))
                .text_color(rgb(kit.palette.text_muted))
                .text_size(px(qol_gpui::theme::TEXT_IDENTITY))
                .child(display_shortcut(mark)),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.activate_option(action, window, cx);
        }));
    kit.row_selected(row, selected)
}

#[cfg(test)]
mod tests {
    use super::{
        display_shortcut_for, help_height, option_actions, OptionAction, HELP_DETAIL_LEFT_ROWS,
        HELP_FLOW_LEFT_ROWS, HELP_FLOW_WINDOW_ROWS, HELP_QUERY_ROWS, HELP_SEARCH_LEFT_ROWS,
        HELP_SEARCH_WINDOW_ROWS,
    };
    use crate::discovery::search::ResultSource;
    use crate::ui::input::InputEffect;
    use crate::ui::layout::{full_window_height, HEADER_HEIGHT};
    use crate::ui::state::LauncherState;
    use gpui::Modifiers;

    #[test]
    fn options_match_selected_result_capabilities() {
        assert_eq!(
            option_actions(None, 0),
            vec![OptionAction::Apps, OptionAction::Files]
        );
        assert_eq!(
            option_actions(Some(ResultSource::File), 0),
            vec![
                OptionAction::Apps,
                OptionAction::Files,
                OptionAction::Open,
                OptionAction::OpenFolder,
            ]
        );
        assert_eq!(
            option_actions(Some(ResultSource::App), 25),
            vec![
                OptionAction::Apps,
                OptionAction::Files,
                OptionAction::Open,
                OptionAction::OpenFolder,
                OptionAction::BoostUp,
                OptionAction::BoostDown,
            ]
        );
    }

    #[test]
    fn displayed_actions_match_launcher_input() {
        let secondary = Modifiers::secondary_key();
        let alt = Modifiers {
            alt: true,
            ..Modifiers::none()
        };
        let shift = Modifiers {
            shift: true,
            ..Modifiers::none()
        };
        for (label, key, modifiers, effect) in [
            ("Open", "enter", Modifiers::none(), InputEffect::Launch),
            (
                "Open containing folder",
                "enter",
                shift,
                InputEffect::OpenFolder,
            ),
            ("Raise rank", "right", secondary, InputEffect::BoostUp),
            ("Raise rank", "right", alt, InputEffect::BoostUp),
            ("Lower rank", "left", secondary, InputEffect::BoostDown),
            ("Lower rank", "left", alt, InputEffect::BoostDown),
            ("Narrow matches", "up", secondary, InputEffect::QueryChanged),
            (
                "Broaden matches",
                "down",
                secondary,
                InputEffect::QueryChanged,
            ),
        ] {
            assert!(HELP_SEARCH_LEFT_ROWS.iter().any(|(name, _)| *name == label));
            assert_eq!(
                LauncherState::new().apply_key(key, &modifiers, 1),
                effect,
                "{label}"
            );
        }
    }

    #[test]
    fn shortcut_labels_use_the_platform_secondary_key() {
        assert_eq!(
            display_shortcut_for("Ctrl+→ / Alt+→", false),
            "Ctrl+→ / Alt+→"
        );
        assert_eq!(
            display_shortcut_for("Ctrl+→ / Alt+→", true),
            "Cmd+→ / Alt+→"
        );
        assert_eq!(display_shortcut_for("Ctrl+C/X/V", true), "Cmd+C/X/V");
    }

    #[test]
    fn help_overlay_fits_the_retained_window() {
        for (left, right, right_extra) in [
            (
                HELP_SEARCH_LEFT_ROWS,
                HELP_QUERY_ROWS,
                HELP_SEARCH_WINDOW_ROWS,
            ),
            (HELP_FLOW_LEFT_ROWS, HELP_QUERY_ROWS, HELP_FLOW_WINDOW_ROWS),
            (HELP_DETAIL_LEFT_ROWS, &[][..], HELP_FLOW_WINDOW_ROWS),
        ] {
            assert!(
                HEADER_HEIGHT + help_height(left, right, right_extra) + 8.0 <= full_window_height()
            );
        }
    }
}
