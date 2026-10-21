use unicode_normalization::char::is_combining_mark;

use crate::listing::{Entry, EntryKind, ItemCount, Size};

const UNREADABLE: &str = "unreadable";

pub fn kind_label(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Folder => "Folder",
        EntryKind::File => "File",
        EntryKind::Link => "Link",
        EntryKind::BrokenLink => "Broken link",
        EntryKind::Pipe => "Pipe",
        EntryKind::Socket => "Socket",
        EntryKind::Device => "Device",
    }
}

pub fn size_label(entry: &Entry, count: Option<ItemCount>, show_hidden: bool) -> String {
    if !entry.opens_as_folder {
        return match entry.size {
            Size::Bytes(bytes) => qol_gpui::format_bytes(bytes),
            Size::Blank => String::new(),
            Size::Unreadable => UNREADABLE.to_string(),
        };
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
    let wanted = crate::nav::letter_count(run);
    let mut seen = 0;
    for (index, letter) in name.char_indices() {
        if seen >= wanted && !is_combining_mark(letter) {
            return Some(index);
        }
        seen += crate::nav::letter_count(letter.encode_utf8(&mut [0; 4]));
    }
    Some(name.len())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{kind_label, size_label, typed_split};
    use crate::listing::{Entry, EntryKind, ItemCount, Size, SortKey};

    fn entry(kind: EntryKind, size: Size, opens_as_folder: bool) -> Entry {
        let name = OsString::from("thing");
        Entry {
            display: "thing".to_string(),
            sort_key: SortKey::new(&name, opens_as_folder),
            name,
            path: PathBuf::from("/thing"),
            id: None,
            kind,
            size,
            hidden: false,
            opens_as_folder,
        }
    }

    #[test]
    fn every_kind_has_its_word() {
        let cases = [
            (EntryKind::Folder, "Folder"),
            (EntryKind::File, "File"),
            (EntryKind::Link, "Link"),
            (EntryKind::BrokenLink, "Broken link"),
            (EntryKind::Pipe, "Pipe"),
            (EntryKind::Socket, "Socket"),
            (EntryKind::Device, "Device"),
        ];
        for (kind, word) in cases {
            assert_eq!(kind_label(kind), word);
        }
    }

    #[test]
    fn files_show_bytes_and_folders_show_their_count_once_it_arrives() {
        let file = entry(EntryKind::File, Size::Bytes(2048), false);
        let folder = entry(EntryKind::Folder, Size::Blank, true);
        let linked_folder = entry(EntryKind::Link, Size::Blank, true);
        let unread = entry(EntryKind::File, Size::Unreadable, false);
        let pipe = entry(EntryKind::Pipe, Size::Blank, false);
        let items = |all, shown| Some(ItemCount::Items { all, shown });
        let cases = [
            (&file, None, qol_gpui::format_bytes(2048)),
            (&unread, None, "unreadable".to_string()),
            (&pipe, None, String::new()),
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
        let folder = entry(EntryKind::Folder, Size::Blank, true);
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
            ("my other", "my o", Some(4)),
            ("Äpfel", "äp", Some("Äp".len())),
            ("apple", "äp", None),
        ];
        for (name, run, expected) in cases {
            assert_eq!(typed_split(name, run), expected, "{name} {run}");
        }
    }
}
