use serde::{Deserialize, Serialize};

use super::{Entry, Size};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    #[default]
    Name,
    Size,
    Modified,
    Kind,
}

impl SortBy {
    pub const MENU: [Self; 4] = [Self::Name, Self::Size, Self::Modified, Self::Kind];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SortOrder {
    pub by: SortBy,
    pub descending: bool,
}

impl SortOrder {
    pub fn new(by: SortBy, descending: bool) -> Self {
        Self { by, descending }
    }

    pub fn chosen(self, by: SortBy) -> Self {
        Self {
            by,
            descending: self.by == by && !self.descending,
        }
    }
}

pub fn arrange(
    entries: &[Entry],
    rows: Vec<usize>,
    order: SortOrder,
    kind: impl Fn(&Entry) -> &'static str,
) -> Vec<usize> {
    if order == SortOrder::default() {
        return rows;
    }
    let (mut folders, mut files): (Vec<usize>, Vec<usize>) = rows
        .into_iter()
        .partition(|&index| entries[index].opens_as_folder);
    match order.by {
        SortBy::Name => {
            if order.descending {
                folders.reverse();
                files.reverse();
            }
        }
        SortBy::Size => sort_on(&mut files, entries, order.descending, |entry| {
            match entry.size {
                Size::Bytes(bytes) => Some(bytes),
                Size::Blank | Size::Unreadable => None,
            }
        }),
        SortBy::Modified => {
            sort_on(&mut folders, entries, order.descending, |entry| {
                entry.modified
            });
            sort_on(&mut files, entries, order.descending, |entry| {
                entry.modified
            });
        }
        SortBy::Kind => {
            sort_on(&mut folders, entries, order.descending, &kind);
            sort_on(&mut files, entries, order.descending, &kind);
        }
    }
    folders.extend(files);
    folders
}

fn sort_on<K: Ord>(
    rows: &mut Vec<usize>,
    entries: &[Entry],
    descending: bool,
    key: impl Fn(&Entry) -> K,
) {
    let mut keyed: Vec<(K, usize)> = rows
        .iter()
        .map(|&index| (key(&entries[index]), index))
        .collect();
    keyed.sort_by(|a, b| {
        let order = a.0.cmp(&b.0);
        if descending {
            order.reverse()
        } else {
            order
        }
    });
    *rows = keyed.into_iter().map(|(_, index)| index).collect();
}
