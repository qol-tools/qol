use std::fs;
use std::io::ErrorKind;

use super::{platform, Amount, Item, Measure, Size, Why};

const PART_LOCKED: &str = "part of it needs an administrator to measure";

pub(super) fn measure(measure: &Measure) -> Amount {
    match measure {
        Measure::Files { dirs, suffix } => files(dirs, suffix),
        Measure::Command { argv, read } => match platform::output(argv) {
            Ok(text) => read(&text).unwrap_or_else(|reason| Amount::unknown(Why::Failed(reason))),
            Err(reason) => Amount::unknown(Why::Failed(reason)),
        },
        Measure::Unknown(reason) => Amount::unknown(Why::Cannot(reason)),
    }
}

fn files(dirs: &[&str], suffix: &str) -> Amount {
    let mut items = Vec::new();
    let mut locked = false;
    for dir in dirs {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                locked = true;
                continue;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Amount::unknown(Why::Failed(format!("{dir}: {error}"))),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_file() && path.to_string_lossy().ends_with(suffix) {
                items.push(Item {
                    name: path.display().to_string(),
                    bytes: Some(meta.len()),
                });
            }
        }
    }
    sized(items, locked)
}

fn sized(items: Vec<Item>, locked: bool) -> Amount {
    let bytes = items.iter().filter_map(|item| item.bytes).sum();
    let size = match (locked, items.is_empty()) {
        (false, _) => Size::Exact(bytes),
        (true, true) => Size::Unknown(Why::NeedsAdmin),
        (true, false) => Size::Estimate {
            bytes,
            why: PART_LOCKED,
        },
    };
    Amount {
        count: items.len(),
        size,
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_only_an_administrator_can_read_leaves_the_size_unknown_or_partial() {
        let item = |bytes| Item {
            name: format!("/var/cache/apt/archives/{bytes}.deb"),
            bytes: Some(bytes),
        };
        assert_eq!(sized(vec![item(3), item(4)], false).size, Size::Exact(7));
        assert_eq!(sized(Vec::new(), true).size, Size::Unknown(Why::NeedsAdmin));
        assert_eq!(
            sized(vec![item(3)], true).size,
            Size::Estimate {
                bytes: 3,
                why: PART_LOCKED
            }
        );
    }

    #[test]
    fn files_are_counted_by_their_ending_and_a_missing_folder_is_empty() {
        let dir = std::env::temp_dir().join(format!("qol-cleanup-measure-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("dir");
        fs::write(dir.join("a.deb"), b"12345").expect("write");
        fs::write(dir.join("lock"), b"").expect("write");
        let path = dir.display().to_string();
        let amount = files(&[path.as_str(), "/nowhere/qol-cleanup"], ".deb");
        assert_eq!(amount.count, 1);
        assert_eq!(amount.size, Size::Exact(5));
        let _ = fs::remove_dir_all(dir);
    }
}
