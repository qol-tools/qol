use gpui::{
    anchored, deferred, px, AnyElement, Context, InteractiveElement, IntoElement, MouseDownEvent,
    ParentElement, Pixels, Point, StatefulInteractiveElement, Styled,
};
use qol_apps::desktop_integration::OpenHandler;
use qol_gpui::kit::kit;
use qol_gpui::theme::{OPACITY_DISABLED, SPACE_INSET};
use qol_gpui::Key;

use super::ExplorerView;

const WIDTH: f32 = 232.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Unpin(usize),
    MoveUp(usize),
    MoveDown(usize),
    Rename(usize),
    TogglePin,
    GoToPath,
    ToggleHidden,
    Settings,
    OpenWith(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Owner {
    Folder,
    Pin(usize),
    OpenWith,
}

#[derive(Clone, Debug)]
pub(super) struct Line {
    command: Command,
    label: String,
    key: Option<Key>,
    enabled: bool,
}

#[derive(Clone, Debug)]
pub(super) enum Item {
    Line(Line),
    Rule,
}

pub(super) struct Menu {
    owner: Owner,
    at: Point<Pixels>,
    items: Vec<Item>,
    selected: usize,
}

impl Line {
    fn new(command: Command, label: impl Into<String>, key: Option<Key>) -> Self {
        Self {
            command,
            label: label.into(),
            key,
            enabled: true,
        }
    }

    fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl Menu {
    pub(super) fn new(owner: Owner, at: Point<Pixels>, items: Vec<Item>) -> Self {
        let mut menu = Self {
            owner,
            at,
            items,
            selected: 0,
        };
        menu.selected = menu.enabled_after(None, true).unwrap_or(0);
        menu
    }

    pub(super) fn pin(
        place: usize,
        name: &str,
        first: bool,
        last: bool,
        at: Point<Pixels>,
    ) -> Self {
        let line = |command, label: String, key| Item::Line(Line::new(command, label, key));
        Self::new(
            Owner::Pin(place),
            at,
            vec![
                line(Command::Unpin(place), format!("Unpin {name}"), None),
                Item::Line(Line::new(Command::MoveUp(place), "Move up", None).enabled(!first)),
                Item::Line(Line::new(Command::MoveDown(place), "Move down", None).enabled(!last)),
                line(Command::Rename(place), "Rename\u{2026}".into(), None),
                Item::Rule,
                line(
                    Command::Settings,
                    "Sidebar settings\u{2026}".into(),
                    Some(Key::symbol(',').secondary()),
                ),
            ],
        )
    }

    pub(super) fn folder(
        folder_name: &str,
        pinned_as: Option<&str>,
        shows_hidden: bool,
        at: Point<Pixels>,
    ) -> Self {
        let pin_label = match pinned_as {
            Some(name) => format!("Unpin {name}"),
            None => format!("Pin {folder_name}"),
        };
        let hidden_label = if shows_hidden {
            "Hide hidden files"
        } else {
            "Show hidden files"
        };
        let line = |command, label: String, key: Key| {
            Item::Line(Line::new(command, label, Some(key.secondary())))
        };
        Self::new(
            Owner::Folder,
            at,
            vec![
                line(Command::TogglePin, pin_label, Key::letter('d')),
                line(
                    Command::GoToPath,
                    "Go to a path\u{2026}".into(),
                    Key::letter('g'),
                ),
                line(Command::ToggleHidden, hidden_label.into(), Key::letter('h')),
                Item::Rule,
                line(
                    Command::Settings,
                    "Explorer settings\u{2026}".into(),
                    Key::symbol(','),
                ),
            ],
        )
    }

    pub(super) fn open_with(handlers: &[OpenHandler], at: Point<Pixels>) -> Self {
        let items = handlers
            .iter()
            .enumerate()
            .map(|(index, handler)| {
                let label = if handler.is_default {
                    format!("{} (default)", handler.name)
                } else {
                    handler.name.clone()
                };
                Item::Line(Line::new(Command::OpenWith(index), label, None))
            })
            .collect();
        Self::new(Owner::OpenWith, at, items)
    }

    pub(super) fn owner(&self) -> Owner {
        self.owner
    }

    pub(super) fn step(&mut self, forward: bool) {
        if let Some(next) = self.enabled_after(Some(self.selected), forward) {
            self.selected = next;
        }
    }

    pub(super) fn selected_command(&self) -> Option<Command> {
        match self.items.get(self.selected) {
            Some(Item::Line(line)) if line.enabled => Some(line.command),
            _ => None,
        }
    }

    fn enabled_after(&self, from: Option<usize>, forward: bool) -> Option<usize> {
        let usable = |index: &usize| matches!(self.items.get(*index), Some(Item::Line(line)) if line.enabled);
        let indices: Vec<usize> = if forward {
            (from.map_or(0, |from| from + 1)..self.items.len()).collect()
        } else {
            (0..from.unwrap_or(0)).rev().collect()
        };
        indices.into_iter().find(usable)
    }
}

pub(super) fn render(menu: &Menu, cx: &mut Context<ExplorerView>) -> AnyElement {
    let kit = kit();
    let rows: Vec<AnyElement> = menu
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| match item {
            Item::Rule => kit.menu_rule().into_any_element(),
            Item::Line(line) => {
                let row = kit.menu_row(
                    ("explorer-menu-line", index),
                    line.label.clone(),
                    line.key,
                    index == menu.selected && line.enabled,
                );
                if !line.enabled {
                    return row.opacity(OPACITY_DISABLED).into_any_element();
                }
                let command = line.command;
                row.cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| view.run_command(command, cx)))
                    .into_any_element()
            }
        })
        .collect();
    let body = kit.menu(WIDTH).children(rows).on_mouse_down_out(
        cx.listener(|view, event: &MouseDownEvent, _, cx| view.close_menu(event, cx)),
    );
    deferred(
        anchored()
            .position(menu.at)
            .snap_to_window_with_margin(px(SPACE_INSET))
            .child(body),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::{point, px};

    use qol_apps::desktop_integration::OpenHandler;

    use super::{Command, Item, Menu, Owner};

    fn at() -> gpui::Point<gpui::Pixels> {
        point(px(0.0), px(0.0))
    }

    #[test]
    fn a_pin_menu_opens_on_unpin_and_steps_over_unavailable_moves() {
        let mut menu = Menu::pin(0, "Docs", true, false, at());
        assert_eq!(menu.selected_command(), Some(Command::Unpin(0)));
        menu.step(true);
        assert_eq!(menu.selected_command(), Some(Command::MoveDown(0)));
        menu.step(true);
        assert_eq!(menu.selected_command(), Some(Command::Rename(0)));
        menu.step(true);
        assert_eq!(menu.selected_command(), Some(Command::Settings));
        menu.step(true);
        assert_eq!(menu.selected_command(), Some(Command::Settings));
        menu.step(false);
        menu.step(false);
        menu.step(false);
        assert_eq!(menu.selected_command(), Some(Command::Unpin(0)));
    }

    #[test]
    fn a_folder_menu_steps_over_its_rule() {
        let mut menu = Menu::folder("Docs", None, false, at());
        assert_eq!(menu.selected_command(), Some(Command::TogglePin));
        menu.step(true);
        menu.step(true);
        menu.step(true);
        assert_eq!(menu.selected_command(), Some(Command::Settings));
    }

    #[test]
    fn a_pin_menu_on_the_last_pin_cannot_move_it_down() {
        let mut menu = Menu::pin(2, "Docs", false, true, at());
        assert_eq!(menu.selected_command(), Some(Command::Unpin(2)));
        menu.step(true);
        assert_eq!(menu.selected_command(), Some(Command::MoveUp(2)));
        menu.step(true);
        assert_eq!(menu.selected_command(), Some(Command::Rename(2)));
        menu.step(false);
        assert_eq!(menu.selected_command(), Some(Command::MoveUp(2)));
    }

    #[test]
    fn a_folder_menu_offers_unpin_when_the_folder_is_pinned() {
        let label = |menu: &Menu| {
            menu.items.iter().find_map(|item| match item {
                Item::Line(line) if line.command == Command::TogglePin => Some(line.label.clone()),
                _ => None,
            })
        };
        let pinned = Menu::folder("Docs", Some("Papers"), false, at());
        assert_eq!(label(&pinned).as_deref(), Some("Unpin Papers"));
        let loose = Menu::folder("Docs", None, false, at());
        assert_eq!(label(&loose).as_deref(), Some("Pin Docs"));
    }

    #[test]
    fn an_open_with_menu_starts_on_the_marked_default() {
        let handlers = [
            OpenHandler {
                id: "a.desktop".into(),
                name: "Editor".into(),
                is_default: true,
            },
            OpenHandler {
                id: "b.desktop".into(),
                name: "Viewer".into(),
                is_default: false,
            },
        ];
        let mut menu = Menu::open_with(&handlers, at());
        let labels: Vec<&str> = menu
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Line(line) => Some(line.label.as_str()),
                Item::Rule => None,
            })
            .collect();
        assert_eq!(labels, ["Editor (default)", "Viewer"]);
        assert_eq!(menu.owner(), Owner::OpenWith);
        assert_eq!(menu.selected_command(), Some(Command::OpenWith(0)));
        menu.step(true);
        assert_eq!(menu.selected_command(), Some(Command::OpenWith(1)));
    }

    #[test]
    fn a_menu_belongs_to_the_button_that_opened_it() {
        let pin = Menu::pin(1, "Docs", false, false, at());
        assert_eq!(pin.owner(), Owner::Pin(1));
        assert_ne!(pin.owner(), Owner::Pin(2));
        assert_ne!(pin.owner(), Owner::Folder);
        let folder = Menu::folder("Docs", None, false, at());
        assert_eq!(folder.owner(), Owner::Folder);
    }
}
