use qol_config::pages::{Control, Enter, Go, Page, Pages, Row};

use crate::scroll_list::SelectionScroll;

use super::super::form_nav::adjacent_visible_row;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Item {
    Header(usize),
    Row(usize, usize),
}

pub(super) fn items(page: &Page) -> Vec<Item> {
    let mut items = Vec::new();
    for (group, rows) in page.groups.iter().enumerate() {
        items.push(Item::Header(group));
        items.extend((0..rows.rows.len()).map(|row| Item::Row(group, row)));
    }
    items
}

pub(super) fn row_at<'a>(page: &'a Page, items: &[Item], index: usize) -> Option<&'a Row> {
    match items.get(index)? {
        Item::Row(group, row) => page.groups.get(*group)?.rows.get(*row),
        Item::Header(_) => None,
    }
}

pub(super) fn selectable(page: &Page, items: &[Item]) -> Vec<usize> {
    (0..items.len())
        .filter(|index| row_at(page, items, *index).is_some_and(|row| !row.info))
        .collect()
}

pub(super) fn header_of(items: &[Item], index: usize) -> Option<usize> {
    items
        .get(..=index)?
        .iter()
        .rposition(|item| matches!(item, Item::Header(_)))
}

pub(super) fn resolve(page: &Page, selected: Option<&str>, previous: usize) -> Option<usize> {
    let items = items(page);
    let selectable = selectable(page, &items);
    if let Some(id) = selected {
        let found = selectable
            .iter()
            .copied()
            .find(|index| row_at(page, &items, *index).is_some_and(|row| row.id == id));
        if found.is_some() {
            return found;
        }
    }
    selectable
        .iter()
        .copied()
        .find(|index| *index >= previous)
        .or_else(|| selectable.last().copied())
}

pub(super) fn settle(page: &Page, place: &Place) -> Option<usize> {
    let wanted = place.selected.as_deref().or(page.focus.as_deref());
    resolve(page, wanted, place.index)
}

pub(super) fn step(page: &Page, current: usize, direction: isize) -> Option<usize> {
    let items = items(page);
    let selectable = selectable(page, &items);
    if selectable.is_empty() {
        return None;
    }
    Some(adjacent_visible_row(&selectable, current, direction))
}

pub(super) fn enter_verb(row: &Row) -> Option<String> {
    match (&row.control, &row.enter) {
        (_, None) => None,
        (Some(Control::Toggle(_)), _) => Some("flip".into()),
        (Some(Control::Button(label)), _) => Some(label.trim_end_matches('…').to_string()),
        (_, Some(Enter::Open(_) | Enter::Replace(_))) => Some("open".into()),
        (_, Some(Enter::Back)) => Some("back".into()),
        (_, Some(Enter::Press)) => Some("choose".into()),
    }
}

#[derive(Clone, Debug)]
pub(super) struct Place {
    pub(super) page: String,
    pub(super) selected: Option<String>,
    pub(super) index: usize,
    pub(super) mark: Option<f32>,
    pub(super) scroll: SelectionScroll,
}

impl Place {
    pub(super) fn new(page: impl Into<String>) -> Self {
        Self {
            page: page.into(),
            selected: None,
            index: 0,
            mark: None,
            scroll: SelectionScroll::new(),
        }
    }
}

#[derive(Debug)]
pub(super) enum Moved {
    Pushed,
    Popped(Vec<Place>),
    Stayed,
}

pub(super) fn go(stack: &mut Vec<Place>, go: &Go, mark: Option<f32>) -> Moved {
    match go {
        Go::Back => pop_to(stack, stack.len().saturating_sub(2)),
        Go::Root => pop_to(stack, 0),
        Go::Open(page) => push(stack, page, mark),
        Go::Replace(page) => {
            if stack.len() > 1 {
                stack.pop();
            }
            push(stack, page, mark)
        }
    }
}

fn push(stack: &mut Vec<Place>, page: &str, mark: Option<f32>) -> Moved {
    if let Some(top) = stack.last_mut() {
        top.mark = mark;
    }
    stack.push(Place::new(page));
    Moved::Pushed
}

pub(super) fn pop_to(stack: &mut Vec<Place>, level: usize) -> Moved {
    if level + 1 >= stack.len() {
        return Moved::Stayed;
    }
    Moved::Popped(stack.split_off(level + 1))
}

pub(super) fn reconcile(stack: &mut Vec<Place>, pages: &Pages) -> bool {
    match stack.first_mut() {
        Some(root) => root.page.clone_from(&pages.root),
        None => stack.push(Place::new(pages.root.clone())),
    }
    let keep = stack
        .iter()
        .position(|place| pages.page(&place.page).is_none())
        .unwrap_or(stack.len())
        .max(1);
    let trimmed = keep < stack.len();
    stack.truncate(keep);
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_config::pages::Group;

    fn page() -> Page {
        Page::new("restart")
            .group(
                Group::new("restart", "1 update finishes.")
                    .row(Row::new("time", "Time").value("01:30").open("time"))
                    .row(Row::new("tonight", "Restart tonight").toggle(false)),
            )
            .group(
                Group::new("what finishes", "installed, waiting.")
                    .row(Row::info("kernel", "Linux kernel 6.8.0-147"))
                    .row(Row::new("now", "Restart now").button("restart…").press()),
            )
    }

    #[test]
    fn items_flatten_headers_before_their_rows() {
        assert_eq!(
            items(&page()),
            [
                Item::Header(0),
                Item::Row(0, 0),
                Item::Row(0, 1),
                Item::Header(1),
                Item::Row(1, 0),
                Item::Row(1, 1),
            ]
        );
        assert_eq!(selectable(&page(), &items(&page())), [1, 2, 5]);
        assert_eq!(header_of(&items(&page()), 5), Some(3));
        assert_eq!(header_of(&items(&page()), 2), Some(0));
    }

    #[test]
    fn selection_follows_the_row_id_then_the_old_place() {
        assert_eq!(resolve(&page(), Some("now"), 0), Some(5));
        assert_eq!(resolve(&page(), Some("gone"), 3), Some(5));
        assert_eq!(resolve(&page(), Some("gone"), 9), Some(5));
        assert_eq!(resolve(&page(), None, 0), Some(1));
        assert_eq!(resolve(&Page::new("empty"), None, 0), None);
    }

    #[test]
    fn a_new_place_opens_on_the_page_focus_and_then_keeps_its_own() {
        let page = page().focus("now");
        let mut place = Place::new("restart");
        assert_eq!(settle(&page, &place), Some(5));
        place.selected = Some("tonight".into());
        assert_eq!(settle(&page, &place), Some(2));
        assert_eq!(
            settle(&page.clone().focus("gone"), &Place::new("restart")),
            Some(1)
        );
    }

    #[test]
    fn steps_skip_headers_and_info_rows_and_stop_at_the_ends() {
        assert_eq!(step(&page(), 2, 1), Some(5));
        assert_eq!(step(&page(), 5, -1), Some(2));
        assert_eq!(step(&page(), 5, 1), Some(5));
        assert_eq!(step(&page(), 1, -1), Some(1));
    }

    #[test]
    fn enter_verbs_name_what_enter_does() {
        let rows: Vec<Row> = page().rows().cloned().collect();
        let verbs: Vec<Option<String>> = rows.iter().map(enter_verb).collect();
        assert_eq!(
            verbs,
            [
                Some("open".to_string()),
                Some("flip".to_string()),
                None,
                Some("restart".to_string()),
            ]
        );
        assert_eq!(
            enter_verb(&Row::new("cancel", "Cancel").back()).as_deref(),
            Some("back")
        );
        assert_eq!(
            enter_verb(&Row::new("c", "02:00").check(true).press()).as_deref(),
            Some("choose")
        );
    }

    fn stack(pages: &[&str]) -> Vec<Place> {
        pages.iter().map(|page| Place::new(*page)).collect()
    }

    fn names(stack: &[Place]) -> Vec<&str> {
        stack.iter().map(|place| place.page.as_str()).collect()
    }

    #[test]
    fn go_moves_the_stack_like_the_reply_says() {
        let mut places = stack(&["main", "restart"]);
        assert!(matches!(
            go(&mut places, &Go::Open("time".into()), Some(80.0)),
            Moved::Pushed
        ));
        assert_eq!(names(&places), ["main", "restart", "time"]);
        assert_eq!(places[1].mark, Some(80.0));

        assert!(
            matches!(go(&mut places, &Go::Back, None), Moved::Popped(gone) if names(&gone) == ["time"])
        );
        assert_eq!(names(&places), ["main", "restart"]);

        go(&mut places, &Go::Replace("ask".into()), None);
        assert_eq!(names(&places), ["main", "ask"]);

        assert!(matches!(go(&mut places, &Go::Root, None), Moved::Popped(_)));
        assert_eq!(names(&places), ["main"]);
        assert!(matches!(go(&mut places, &Go::Back, None), Moved::Stayed));
        assert_eq!(names(&places), ["main"]);
    }

    #[test]
    fn reconcile_drops_pages_that_vanished_and_keeps_the_root() {
        let mut pages = Pages::new("main");
        pages.insert("main", Page::new("Update Manager"));
        pages.insert("waiting", Page::new("waiting"));
        let mut places = stack(&["main", "waiting", "update:urllib3", "install:urllib3"]);
        assert!(reconcile(&mut places, &pages));
        assert_eq!(names(&places), ["main", "waiting"]);
        assert!(!reconcile(&mut places, &pages));

        let mut empty = Vec::new();
        reconcile(&mut empty, &pages);
        assert_eq!(names(&empty), ["main"]);

        let mut lost = stack(&["main", "waiting"]);
        assert!(reconcile(&mut lost, &Pages::new("main")));
        assert_eq!(names(&lost), ["main"]);
    }
}
