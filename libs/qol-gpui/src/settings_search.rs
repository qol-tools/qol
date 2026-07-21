use std::rc::Rc;

use gpui::*;

use crate::monitor::MonitorTracker;
use crate::scroll_list::ScrollList;
use crate::surface::{Anchor, Surface, SurfaceDismisser, SurfaceKind};
use crate::theme::{settings_panel_runtime, SettingsPanelPalette};

const SEARCH_WIDTH: f32 = 560.0;
const SEARCH_HEIGHT: f32 = 456.0;
const MAX_VISIBLE: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsSearchItem {
    pub plugin_id: String,
    pub plugin_name: String,
    pub config_key: String,
    pub section: Option<String>,
    pub label: String,
}

impl SettingsSearchItem {
    fn search_text(&self) -> String {
        format!(
            "{} {} {} {}",
            self.label,
            self.section.as_deref().unwrap_or_default(),
            self.plugin_name,
            self.config_key
        )
    }

    fn path(&self) -> String {
        match self.section.as_deref() {
            Some(section) => format!("{} › {section}", self.plugin_name),
            None => self.plugin_name.clone(),
        }
    }
}

type SelectHandler = dyn Fn(SettingsSearchItem, &mut App);

pub fn open(
    items: Vec<SettingsSearchItem>,
    tracker: &MonitorTracker,
    on_select: impl Fn(SettingsSearchItem, &mut App) + 'static,
    cx: &mut App,
) -> anyhow::Result<SurfaceDismisser> {
    Surface::new(SurfaceKind::Panel)
        .title("Search Settings")
        .anchor(Anchor::MonitorCenter)
        .size(size(px(SEARCH_WIDTH), px(SEARCH_HEIGHT)))
        .show_focused(tracker, cx, move |dismisser, _window, cx| {
            SettingsSearchView::new(items, dismisser, Rc::new(on_select), cx)
        })
}

struct SettingsSearchView {
    items: Vec<SettingsSearchItem>,
    matches: Vec<usize>,
    query: String,
    list: ScrollList,
    dismisser: SurfaceDismisser,
    on_select: Rc<SelectHandler>,
    palette: SettingsPanelPalette,
    focus_handle: FocusHandle,
}

impl SettingsSearchView {
    fn new(
        items: Vec<SettingsSearchItem>,
        dismisser: SurfaceDismisser,
        on_select: Rc<SelectHandler>,
        cx: &mut Context<Self>,
    ) -> Self {
        let matches = matching_indices(&items, "");
        let mut list = ScrollList::new(MAX_VISIBLE);
        list.sync(matches.len());
        Self {
            items,
            matches,
            query: String::new(),
            list,
            dismisser,
            on_select,
            palette: settings_panel_runtime(),
            focus_handle: cx.focus_handle(),
        }
    }

    fn handle_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "escape" | "esc" => {
                self.dismisser.dismiss(cx);
                return;
            }
            "up" => self.list.move_up(),
            "down" => self.list.move_down(self.matches.len()),
            "enter" | "return" => {
                self.activate_selected(cx);
                return;
            }
            "backspace" => {
                self.query.pop();
                self.refresh_matches();
            }
            _ => {
                if event.keystroke.modifiers.secondary()
                    || event.keystroke.modifiers.control
                    || event.keystroke.modifiers.alt
                {
                    return;
                }
                let Some(value) = event.keystroke.key_char.as_deref() else {
                    return;
                };
                self.query.push_str(value);
                self.refresh_matches();
            }
        }
        self.list.sync(self.matches.len());
        cx.notify();
    }

    fn refresh_matches(&mut self) {
        self.matches = matching_indices(&self.items, &self.query);
        self.list.reset();
        self.list.sync(self.matches.len());
    }

    fn activate_selected(&mut self, cx: &mut App) {
        let Some(item) = self
            .matches
            .get(self.list.selected)
            .and_then(|index| self.items.get(*index))
            .cloned()
        else {
            return;
        };
        self.dismisser.dismiss(cx);
        (self.on_select)(item, cx);
    }

    fn render_row(&self, result_index: usize, cx: &mut Context<Self>) -> AnyElement {
        let item_index = self.matches[result_index];
        let item = &self.items[item_index];
        let selected = result_index == self.list.selected;
        let label_color = if selected {
            self.palette.section_text
        } else {
            self.palette.label_text
        };
        let path_color = if selected {
            self.palette.label_text
        } else {
            self.palette.status_muted
        };
        let mut row = div()
            .id(("settings-search-result", result_index))
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(self.palette.panel_border))
            .cursor_pointer()
            .text_color(rgb(label_color))
            .child(item.label.clone())
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(path_color))
                    .child(item.path()),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                this.list.selected = result_index;
                this.activate_selected(cx);
            }));
        if selected {
            row = row
                .bg(rgb(self.palette.row_bg_selected))
                .border_color(rgb(self.palette.row_border_selected));
        }
        row.into_any_element()
    }
}

impl Focusable for SettingsSearchView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsSearchView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = if self.query.is_empty() {
            "Type a setting name…".to_string()
        } else {
            format!("{}▍", self.query)
        };
        let rows = self
            .list
            .visible_range(self.matches.len())
            .map(|index| self.render_row(index, cx))
            .collect::<Vec<_>>();
        let empty = self.matches.is_empty().then(|| {
            div()
                .px_3()
                .py_4()
                .text_color(rgb(self.palette.status_muted))
                .child("No matching settings")
        });
        div()
            .id("settings-search")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key))
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .bg(rgb(self.palette.window_bg))
            .border_1()
            .border_color(rgb(self.palette.panel_border))
            .text_color(rgb(self.palette.section_text))
            .child(div().text_xl().child("Search Settings"))
            .child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(self.palette.row_border_selected))
                    .bg(rgb(self.palette.dropdown_bg))
                    .text_color(rgb(if self.query.is_empty() {
                        self.palette.status_muted
                    } else {
                        self.palette.section_text
                    }))
                    .child(query),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(rows)
                    .children(empty),
            )
            .child(
                div()
                    .mt_auto()
                    .text_sm()
                    .text_color(rgb(self.palette.status_muted))
                    .child("↑↓ navigate · Enter open · Esc close"),
            )
    }
}

fn matching_indices(items: &[SettingsSearchItem], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..items.len()).collect();
    }
    let mut matches = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            qol_search::fuzzy_match(query, &item.search_text())
                .map(|matched| (index, matched.score))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|(index, score)| (*score, *index));
    matches.into_iter().map(|(index, _)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::{matching_indices, SettingsSearchItem};

    fn item(
        plugin_name: &str,
        section: Option<&str>,
        label: &str,
        config_key: &str,
    ) -> SettingsSearchItem {
        SettingsSearchItem {
            plugin_id: plugin_name.to_lowercase(),
            plugin_name: plugin_name.into(),
            config_key: config_key.into(),
            section: section.map(str::to_string),
            label: label.into(),
        }
    }

    #[test]
    fn search_matches_labels_sections_plugins_and_config_keys() {
        let items = [
            item(
                "Bluetooth",
                Some("Reconnection"),
                "Initial retry delay",
                "retry_initial_seconds",
            ),
            item(
                "QoL Shot",
                Some("Capture"),
                "Pinned border",
                "capture.pin_border",
            ),
        ];
        let cases = [
            ("retry delay", vec![0]),
            ("reconnection", vec![0]),
            ("bluetooth", vec![0]),
            ("pin_border", vec![1]),
            ("missing", vec![]),
            ("", vec![0, 1]),
        ];
        for (query, expected) in cases {
            assert_eq!(matching_indices(&items, query), expected, "query={query}");
        }
    }
}
