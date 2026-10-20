use unicode_normalization::char::is_combining_mark;

use crate::listing::{Entry, EntryKind, ItemCount};

const UNREADABLE: &str = "unreadable";

pub fn kind_label(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Folder => "Folder",
        EntryKind::File => "File",
        EntryKind::Link => "Link",
    }
}

pub fn size_label(entry: &Entry, count: Option<ItemCount>, show_hidden: bool) -> String {
    if !entry.opens_as_folder {
        return entry
            .size
            .map_or_else(|| UNREADABLE.to_string(), qol_gpui::format_bytes);
    }
    match count {
        Some(ItemCount::Items { all, shown }) => qol_gpui::plural(
            usize::try_from(if show_hidden { all } else { shown }).unwrap_or(usize::MAX),
            "item",
            "items",
        ),
        Some(ItemCount::Unreadable) => UNREADABLE.to_string(),
        None => String::new(),
    }
}

pub fn typed_split(name: &str, run: &str) -> Option<usize> {
    if run.is_empty() || !crate::nav::starts_with_ignoring_case(name, run) {
        return None;
    }
    let wanted = crate::nav::folded(run).count();
    let mut seen = 0;
    for (index, letter) in name.char_indices() {
        if seen >= wanted && !is_combining_mark(letter) {
            return Some(index);
        }
        seen += crate::nav::folded(letter.encode_utf8(&mut [0; 4])).count();
    }
    Some(name.len())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{kind_label, size_label, typed_split};
    use crate::listing::{Entry, EntryKind, ItemCount, SortKey};

    fn entry(kind: EntryKind, size: u64, opens_as_folder: bool) -> Entry {
        let name = OsString::from("thing");
        Entry {
            display: "thing".to_string(),
            sort_key: SortKey::new(&name, opens_as_folder),
            name,
            path: PathBuf::from("/thing"),
            id: None,
            kind,
            size: Some(size),
            hidden: false,
            opens_as_folder,
        }
    }

    #[test]
    fn every_kind_has_its_word() {
        assert_eq!(kind_label(EntryKind::Folder), "Folder");
        assert_eq!(kind_label(EntryKind::File), "File");
        assert_eq!(kind_label(EntryKind::Link), "Link");
    }

    #[test]
    fn files_show_bytes_and_folders_show_their_count_once_it_arrives() {
        let file = entry(EntryKind::File, 2048, false);
        let folder = entry(EntryKind::Folder, 0, true);
        let linked_folder = entry(EntryKind::Link, 0, true);
        let unread = Entry {
            size: None,
            ..entry(EntryKind::File, 0, false)
        };
        let items = |all, shown| Some(ItemCount::Items { all, shown });
        let cases = [
            (&file, None, qol_gpui::format_bytes(2048)),
            (&unread, None, "unreadable".to_string()),
            (&folder, None, String::new()),
            (&folder, items(1, 1), "1 item".to_string()),
            (&folder, items(12, 12), "12 items".to_string()),
            (
                &folder,
                Some(ItemCount::Unreadable),
                "unreadable".to_string(),
            ),
            (&linked_folder, None, String::new()),
            (&linked_folder, items(2, 2), "2 items".to_string()),
        ];
        for (entry, count, expected) in cases {
            assert_eq!(
                size_label(entry, count, false),
                expected,
                "{entry:?} {count:?}"
            );
        }
    }

    #[test]
    fn folder_counts_leave_out_hidden_items_while_they_are_hidden() {
        let folder = entry(EntryKind::Folder, 0, true);
        let count = Some(ItemCount::Items { all: 5, shown: 3 });
        assert_eq!(size_label(&folder, count, false), "3 items");
        assert_eq!(size_label(&folder, count, true), "5 items");
    }

    #[test]
    fn the_typed_run_splits_the_name_where_the_run_ends() {
        let cases = [
            ("Documents", "doc", Some(3)),
            ("Documents", "DOCUMENTS", Some(9)),
            ("Documents", "", None),
            ("Documents", "dox", None),
            ("Ärger.txt", "är", Some("Är".len())),
            ("notes", "notes.md", None),
            ("éclair.txt", "ec", Some("éc".len())),
            ("E\u{301}mile", "em", Some("E\u{301}m".len())),
            ("E\u{301}mile", "e", Some("E\u{301}".len())),
        ];
        for (name, run, expected) in cases {
            assert_eq!(typed_split(name, run), expected, "{name} {run}");
        }
    }
}
