use unicode_normalization::char::is_combining_mark;

use crate::listing::{Entry, EntryKind, ItemCount};

pub fn kind_label(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Folder => "Folder",
        EntryKind::File => "File",
        EntryKind::Link => "Link",
    }
}

pub fn size_label(entry: &Entry, count: Option<ItemCount>) -> String {
    if !entry.opens_as_folder {
        return qol_gpui::format_bytes(entry.size);
    }
    match count {
        Some(ItemCount::Items(items)) => qol_gpui::plural(
            usize::try_from(items).unwrap_or(usize::MAX),
            "item",
            "items",
        ),
        Some(ItemCount::Unreadable) => "unreadable".to_string(),
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
            kind,
            size,
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
        let cases = [
            (&file, None, qol_gpui::format_bytes(2048)),
            (&folder, None, String::new()),
            (&folder, Some(ItemCount::Items(1)), "1 item".to_string()),
            (&folder, Some(ItemCount::Items(12)), "12 items".to_string()),
            (
                &folder,
                Some(ItemCount::Unreadable),
                "unreadable".to_string(),
            ),
            (&linked_folder, None, String::new()),
        ];
        for (entry, count, expected) in cases {
            assert_eq!(size_label(entry, count), expected, "{entry:?} {count:?}");
        }
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
