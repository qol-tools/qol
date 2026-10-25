use gpui::{
    anchored, deferred, px, AnyElement, Context, InteractiveElement, IntoElement, MouseDownEvent,
    ParentElement, Pixels, Point, StatefulInteractiveElement, Styled,
};
use qol_gpui::kit::kit;
use qol_gpui::theme::{OPACITY_DISABLED, SPACE_INSET};
use qol_gpui::Key;

use super::ExplorerView;
use crate::listing::{SortBy, SortOrder};

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
    Sort(SortBy),
    Settings,
    OpenItem,
    Cut,
    Copy,
    Paste,
    RenameItem,
    Trash,
    NewFolder,
    Undo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Owner {
    Folder,
    Pin(usize),
    Item,
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

fn entry(command: Command, label: &str, key: Option<Key>, enabled: bool) -> Item {
    Item::Line(Line::new(command, label, key).enabled(enabled))
}

fn secondary(key: Key) -> Option<Key> {
    Some(key.secondary())
}

fn paste_line(can_paste: bool) -> Item {
    entry(
        Command::Paste,
        "Paste",
        secondary(Key::letter('v')),
        can_paste,
    )
}

fn make_lines() -> [Item; 2] {
    [
        entry(
            Command::NewFolder,
            "New folder",
            secondary(Key::letter('n').shift()),
            true,
        ),
        entry(Command::Undo, "Undo", secondary(Key::letter('z')), true),
    ]
}

fn edit_lines(can_paste: bool) -> Vec<Item> {
    let mut lines = vec![paste_line(can_paste)];
    lines.extend(make_lines());
    lines
}

fn item_lines(can_paste: bool, single: bool) -> Vec<Item> {
    let mut lines = vec![
        entry(Command::OpenItem, "Open", Some(Key::ENTER), true),
        entry(Command::Cut, "Cut", secondary(Key::letter('x')), true),
        entry(Command::Copy, "Copy", secondary(Key::letter('c')), true),
        paste_line(can_paste),
        entry(Command::RenameItem, "Rename", Key::parse("f2"), single),
        entry(Command::Trash, "Move to trash", Some(Key::DELETE), true),
        Item::Rule,
    ];
    lines.extend(make_lines());
    lines
}

fn folder_lines(
    folder_name: &str,
    pinned_as: Option<&str>,
    shows_hidden: bool,
    order: SortOrder,
) -> Vec<Item> {
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
    let mut items = vec![
        line(Command::TogglePin, pin_label, Key::letter('d')),
        line(
            Command::GoToPath,
            "Go to a path\u{2026}".into(),
            Key::letter('g'),
        ),
        line(Command::ToggleHidden, hidden_label.into(), Key::letter('h')),
        Item::Rule,
    ];
    items.extend(SortBy::MENU.into_iter().zip(1..).map(|(by, digit)| {
        Item::Line(Line::new(
            Command::Sort(by),
            sort_label(by, order),
            Key::parse(&format!("alt+{digit}")).map(Key::secondary),
        ))
    }));
    items.push(Item::Rule);
    items.push(line(
        Command::Settings,
        "Explorer settings\u{2026}".into(),
        Key::symbol(','),
    ));
    items
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
        order: SortOrder,
        at: Point<Pixels>,
    ) -> Self {
        Self::new(
            Owner::Folder,
            at,
            folder_lines(folder_name, pinned_as, shows_hidden, order),
        )
    }

    pub(super) fn background(
        folder_name: &str,
        pinned_as: Option<&str>,
        shows_hidden: bool,
        order: SortOrder,
        can_paste: bool,
        at: Point<Pixels>,
    ) -> Self {
        let mut items = edit_lines(can_paste);
        items.push(Item::Rule);
        items.extend(folder_lines(folder_name, pinned_as, shows_hidden, order));
        Self::new(Owner::Folder, at, items)
    }

    pub(super) fn item(can_paste: bool, single: bool, at: Point<Pixels>) -> Self {
        Self::new(Owner::Item, at, item_lines(can_paste, single))
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

fn sort_label(by: SortBy, order: SortOrder) -> String {
    let name = match by {
        SortBy::Name => "name",
        SortBy::Size => "size",
        SortBy::Modified => "date modified",
        SortBy::Kind => "kind",
    };
    if order.by != by {
        return format!("Sort by {name}");
    }
    let direction = if order.descending {
        "descending"
    } else {
        "ascending"
    };
    format!("Sort by {name}, {direction}")
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

    use super::{Command, Item, Menu, Owner};
    use crate::listing::{SortBy, SortOrder};

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
        let mut menu = Menu::folder("Docs", None, false, SortOrder::default(), at());
        assert_eq!(menu.selected_command(), Some(Command::TogglePin));
        menu.step(true);
        menu.step(true);
        menu.step(true);
        assert_eq!(menu.selected_command(), Some(Command::Sort(SortBy::Name)));
        for _ in 0..3 {
            menu.step(true);
        }
        assert_eq!(menu.selected_command(), Some(Command::Sort(SortBy::Kind)));
        menu.step(true);
        assert_eq!(menu.selected_command(), Some(Command::Settings));
    }

    #[test]
    fn the_active_sort_line_names_its_direction() {
        let labels = |order: SortOrder| -> Vec<String> {
            Menu::folder("Docs", None, false, order, at())
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Line(line) if matches!(line.command, Command::Sort(_)) => {
                        Some(line.label.clone())
                    }
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            labels(SortOrder::default()),
            [
                "Sort by name, ascending",
                "Sort by size",
                "Sort by date modified",
                "Sort by kind"
            ]
        );
        assert_eq!(
            labels(SortOrder::new(SortBy::Modified, true)),
            [
                "Sort by name",
                "Sort by size",
                "Sort by date modified, descending",
                "Sort by kind"
            ]
        );
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
        let pinned = Menu::folder("Docs", Some("Papers"), false, SortOrder::default(), at());
        assert_eq!(label(&pinned).as_deref(), Some("Unpin Papers"));
        let loose = Menu::folder("Docs", None, false, SortOrder::default(), at());
        assert_eq!(label(&loose).as_deref(), Some("Pin Docs"));
    }

    #[test]
    fn a_menu_belongs_to_the_button_that_opened_it() {
        let pin = Menu::pin(1, "Docs", false, false, at());
        assert_eq!(pin.owner(), Owner::Pin(1));
        assert_ne!(pin.owner(), Owner::Pin(2));
        assert_ne!(pin.owner(), Owner::Folder);
        let folder = Menu::folder("Docs", None, false, SortOrder::default(), at());
        assert_eq!(folder.owner(), Owner::Folder);
    }

    fn lines(menu: &Menu) -> Vec<(Command, bool)> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                Item::Line(line) => Some((line.command, line.enabled)),
                Item::Rule => None,
            })
            .collect()
    }

    #[test]
    fn an_item_menu_lists_the_file_actions_with_paste_and_rename_gated() {
        let cases = [
            (true, true, true, true),
            (false, true, false, true),
            (true, false, true, false),
            (false, false, false, false),
        ];
        for (can_paste, single, paste, rename) in cases {
            let menu = Menu::item(can_paste, single, at());
            assert_eq!(
                lines(&menu),
                vec![
                    (Command::OpenItem, true),
                    (Command::Cut, true),
                    (Command::Copy, true),
                    (Command::Paste, paste),
                    (Command::RenameItem, rename),
                    (Command::Trash, true),
                    (Command::NewFolder, true),
                    (Command::Undo, true),
                ],
                "{can_paste} {single}"
            );
            assert_eq!(menu.owner(), Owner::Item);
        }
    }

    #[test]
    fn an_item_menu_opens_on_open_and_steps_over_what_is_unavailable() {
        let mut menu = Menu::item(false, false, at());
        let mut walked = vec![menu.selected_command()];
        for _ in 0..5 {
            menu.step(true);
            walked.push(menu.selected_command());
        }
        assert_eq!(
            walked,
            [
                Some(Command::OpenItem),
                Some(Command::Cut),
                Some(Command::Copy),
                Some(Command::Trash),
                Some(Command::NewFolder),
                Some(Command::Undo),
            ]
        );
        menu.step(true);
        assert_eq!(menu.selected_command(), Some(Command::Undo));
    }

    #[test]
    fn a_background_menu_puts_the_edit_lines_before_the_folder_lines() {
        let menu = Menu::background("Docs", None, true, SortOrder::default(), false, at());
        assert_eq!(
            lines(&menu),
            vec![
                (Command::Paste, false),
                (Command::NewFolder, true),
                (Command::Undo, true),
                (Command::TogglePin, true),
                (Command::GoToPath, true),
                (Command::ToggleHidden, true),
                (Command::Sort(SortBy::Name), true),
                (Command::Sort(SortBy::Size), true),
                (Command::Sort(SortBy::Modified), true),
                (Command::Sort(SortBy::Kind), true),
                (Command::Settings, true),
            ]
        );
        assert_eq!(menu.selected_command(), Some(Command::NewFolder));
        assert_eq!(menu.owner(), Owner::Folder);
        let held = Menu::background("Docs", None, true, SortOrder::default(), true, at());
        assert_eq!(held.selected_command(), Some(Command::Paste));
    }
}
