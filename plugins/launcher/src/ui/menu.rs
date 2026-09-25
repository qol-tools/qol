use gpui::*;

use super::layout::{HEADER_HEIGHT, WINDOW_WIDTH};
use super::trace;
use super::LauncherView;
use crate::discovery::search::{ResultSource, SearchMode};
use qol_gpui::icon::{icon, Icon};
use qol_gpui::Key;

const HELP_ROW_HEIGHT: f32 = 17.0;
type HelpRows = &'static [(&'static str, &'static [Key])];

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

    fn shortcut(self) -> &'static [Key] {
        match self {
            Self::Open => &[Key::ENTER],
            Self::OpenFolder => const { &[Key::ENTER.shift()] },
            Self::BoostUp => const { &[Key::RIGHT.secondary(), Key::RIGHT.alt()] },
            Self::BoostDown => const { &[Key::LEFT.secondary(), Key::LEFT.alt()] },
            Self::Apps | Self::Files => &[],
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
    ("Open", &[Key::ENTER]),
    ("Open folder", &[Key::ENTER.shift()]),
    ("Apps or files", &[Key::TAB]),
    ("Options", &[Key::ENTER.alt()]),
    ("Close", &[Key::ESC]),
];

const HELP_SEARCH_RIGHT_ROWS: HelpRows = &[
    ("Raise rank in list", &[Key::RIGHT.secondary()]),
    ("Lower rank in list", &[Key::LEFT.secondary()]),
    ("Narrow", &[Key::UP.secondary()]),
    ("Broaden", &[Key::DOWN.secondary()]),
];

const HELP_QUERY_ROWS: HelpRows = &[
    ("Query", &[]),
    ("Move caret", &[Key::LEFT, Key::RIGHT]),
    ("Select text", &[Key::LEFT.shift(), Key::RIGHT.shift()]),
    ("Start / end", &[Key::HOME, Key::END]),
    ("Delete text", &[Key::BACKSPACE, Key::DELETE]),
    ("Select all", &[Key::letter('a').secondary()]),
    (
        "Copy / cut / paste",
        &[
            Key::letter('c').secondary(),
            Key::letter('x').secondary(),
            Key::letter('v').secondary(),
        ],
    ),
];

const HELP_FLOW_LEFT_ROWS: HelpRows = &[
    ("Flow results", &[]),
    ("Move through results", &[Key::UP, Key::DOWN]),
    ("Open detail", &[Key::ENTER]),
    ("Dislike result", &[Key::letter('x').alt()]),
    ("Back to search", &[Key::ESC]),
];

const HELP_DETAIL_LEFT_ROWS: HelpRows = &[
    ("Flow detail", &[]),
    ("Scroll", &[Key::UP, Key::DOWN]),
    ("Activate", &[Key::ENTER]),
    ("Back to results", &[Key::ESC]),
];

const HELP_FLOW_WINDOW_ROWS: HelpRows = &[("Window", &[]), ("Help", &[Key::letter('h').alt()])];

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
        (HELP_SEARCH_LEFT_ROWS, HELP_SEARCH_RIGHT_ROWS, &[])
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
        let panel = div()
            .id("launcher-menu")
            .absolute()
            .h(px(self.menu_height()))
            .p(px(qol_gpui::theme::SPACE_TIGHT))
            .track_scroll(&self.menu_scroll)
            .overflow_y_scroll()
            .flex()
            .flex_col();
        let mut panel = if kind == MenuKind::Help {
            panel
                .top(px(HEADER_HEIGHT))
                .left_0()
                .w(px(WINDOW_WIDTH))
                .px(px(qol_gpui::theme::SPACE_PAD))
                .border_b(px(1.0))
                .border_color(rgba(kit.washes.hairline.packed()))
                .bg(super::view::bg_color())
        } else {
            panel
                .top(px(HEADER_HEIGHT + 4.0))
                .right(px(8.0))
                .w(px(280.0))
                .border(px(1.0))
                .border_color(rgba(kit.washes.hairline_strong.packed()))
                .bg(rgb(kit.palette.surface_raised))
                .shadow(qol_gpui::kit::float_shadow(kit.palette.text_primary))
        };
        if kind == MenuKind::Help {
            let (left, right, right_extra) = self.help_rows();
            panel = panel.child(menu_title("Keys", Key::letter('h').alt()));
            panel = panel.child(
                div()
                    .flex()
                    .gap(px(qol_gpui::theme::SPACE_INSET))
                    .child(help_column(left, &[]))
                    .child(help_column(right, right_extra)),
            );
            return panel;
        }
        panel = panel.child(menu_title("Options", Key::ENTER.alt()));
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

fn menu_title(label: &str, shortcut: Key) -> Div {
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
        .child(kit.key_name(
            &[shortcut],
            qol_gpui::theme::TEXT_IDENTITY,
            kit.palette.text_muted,
        ))
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

fn help_row(label: &str, shortcut: &[Key]) -> Div {
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
        .child(kit.key_name(
            shortcut,
            qol_gpui::theme::TEXT_IDENTITY,
            kit.palette.text_muted,
        ))
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

fn option_row(
    action: OptionAction,
    selected: bool,
    mode: SearchMode,
    cx: &mut Context<LauncherView>,
) -> impl IntoElement {
    let kit = qol_gpui::kit::kit();
    let ink = kit.highlight_ground(selected).faint;
    let chosen = match action {
        OptionAction::Apps => mode == SearchMode::Apps,
        OptionAction::Files => mode == SearchMode::Files,
        _ => false,
    };
    let mark = if chosen {
        icon(
            Icon::Tick,
            qol_gpui::theme::TEXT_IDENTITY,
            kit.highlight_ground(selected).ink,
        )
        .into_any_element()
    } else {
        kit.key_name(action.shortcut(), qol_gpui::theme::TEXT_IDENTITY, ink)
            .into_any_element()
    };
    let row = div()
        .id(SharedString::from(format!("launcher-option-{action:?}")))
        .h(px(qol_gpui::theme::HEIGHT_INLINE))
        .px(px(qol_gpui::theme::SPACE_INSET))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(qol_gpui::theme::SPACE_SNUG))
        .cursor_pointer()
        .text_color(rgb(kit.palette.text_primary))
        .text_size(px(qol_gpui::theme::TEXT_MICRO))
        .child(div().flex_1().min_w_0().truncate().child(action.label()))
        .child(mark)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.activate_option(action, window, cx);
        }));
    kit.highlight(row, selected)
}

#[cfg(test)]
mod tests {
    use super::{
        help_height, option_actions, OptionAction, HELP_DETAIL_LEFT_ROWS, HELP_FLOW_LEFT_ROWS,
        HELP_FLOW_WINDOW_ROWS, HELP_QUERY_ROWS, HELP_SEARCH_LEFT_ROWS, HELP_SEARCH_RIGHT_ROWS,
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
            ("Open folder", "enter", shift, InputEffect::OpenFolder),
            (
                "Raise rank in list",
                "right",
                secondary,
                InputEffect::BoostUp,
            ),
            ("Raise rank in list", "right", alt, InputEffect::BoostUp),
            (
                "Lower rank in list",
                "left",
                secondary,
                InputEffect::BoostDown,
            ),
            ("Lower rank in list", "left", alt, InputEffect::BoostDown),
            ("Narrow", "up", secondary, InputEffect::QueryChanged),
            ("Broaden", "down", secondary, InputEffect::QueryChanged),
        ] {
            assert!(HELP_SEARCH_LEFT_ROWS
                .iter()
                .chain(HELP_SEARCH_RIGHT_ROWS)
                .any(|(name, _)| *name == label));
            let mut state = LauncherState::new();
            state.list_focused = true;
            assert_eq!(state.apply_key(key, &modifiers, 1), effect, "{label}");
        }
    }

    #[test]
    fn help_overlay_fits_the_retained_window() {
        for (left, right, right_extra) in [
            (HELP_SEARCH_LEFT_ROWS, HELP_SEARCH_RIGHT_ROWS, &[][..]),
            (HELP_FLOW_LEFT_ROWS, HELP_QUERY_ROWS, HELP_FLOW_WINDOW_ROWS),
            (HELP_DETAIL_LEFT_ROWS, &[][..], HELP_FLOW_WINDOW_ROWS),
        ] {
            assert!(
                HEADER_HEIGHT + help_height(left, right, right_extra) + 8.0 <= full_window_height()
            );
        }
    }
}
