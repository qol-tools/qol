use std::fs;
use std::io::{Error, ErrorKind};

use super::{platform, Amount, Item, Measure, Size, Why};

pub(super) fn measure(measure: &Measure) -> Amount {
    match measure {
        Measure::Files { dir, suffix } => files(dir, suffix),
        Measure::Command { argv, read } => match platform::output(argv) {
            Ok(text) => read(&text).unwrap_or_else(|reason| Amount::unknown(Why::Failed(reason))),
            Err(reason) => Amount::unknown(Why::Failed(reason)),
        },
        Measure::Unknown(reason) => Amount::unknown(Why::Cannot(reason)),
    }
}

fn files(dir: &str, suffix: &str) -> Amount {
    let unread = |error: Error| match error.kind() {
        ErrorKind::PermissionDenied => Amount::unknown(Why::NeedsAdmin),
        _ => Amount::unknown(Why::Failed(format!("{dir}: {error}"))),
    };
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return sized(Vec::new()),
        Err(error) => return unread(error),
    };
    let mut items = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return unread(error),
        };
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return unread(error),
        };
        let path = entry.path();
        if meta.is_file() && path.to_string_lossy().ends_with(suffix) {
            items.push(Item {
                name: path.display().to_string(),
                bytes: Some(meta.len()),
            });
        }
    }
    sized(items)
}

fn sized(items: Vec<Item>) -> Amount {
    Amount {
        count: items.len(),
        present: items.len(),
        size: Size::Exact(items.iter().filter_map(|item| item.bytes).sum()),
        items,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("qol-cleanup-measure-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("dir");
        dir
    }

    #[test]
    fn files_are_counted_by_their_ending_and_a_missing_folder_is_empty() {
        let dir = folder("readable");
        fs::write(dir.join("a.deb"), b"12345").expect("write");
        fs::write(dir.join("lock"), b"").expect("write");
        let amount = files(&dir.display().to_string(), ".deb");
        assert_eq!(amount.count, 1);
        assert_eq!(amount.size, Size::Exact(5));
        let missing = files("/nowhere/qol-cleanup", ".deb");
        assert_eq!((missing.count, missing.size), (0, Size::Exact(0)));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_only_an_administrator_can_read_leaves_the_size_unknown() {
        use std::os::unix::fs::PermissionsExt;
        let dir = folder("locked");
        fs::write(dir.join("a.deb"), b"12345").expect("write");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).expect("lock");
        let readable = fs::read_dir(&dir).is_ok();
        let amount = files(&dir.display().to_string(), ".deb");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("unlock");
        let _ = fs::remove_dir_all(&dir);
        if readable {
            return;
        }
        assert_eq!(amount.size, Size::Unknown(Why::NeedsAdmin));
        assert_eq!(amount.count, 0);
    }
}
