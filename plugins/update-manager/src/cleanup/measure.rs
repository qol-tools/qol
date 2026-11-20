use std::fs;
use std::io::{Error, ErrorKind};

use super::{Amount, Item, Measure, Size, Why};

pub(super) type Output = dyn FnMut(&'static [&'static str]) -> Result<String, String>;

pub(super) fn measure_by(measure: &Measure, output: &mut Output) -> Amount {
    match measure {
        Measure::Files { dir, suffix } => files(dir, suffix),
        Measure::Command { argv, read } => read_output(output(argv), *read),
        Measure::Unknown(reason) => Amount::unknown(Why::Cannot(reason)),
    }
}

pub(super) fn once(
    mut run: impl FnMut(&'static [&'static str]) -> Result<String, String>,
) -> impl FnMut(&'static [&'static str]) -> Result<String, String> {
    let mut ran: Vec<(&'static [&'static str], Result<String, String>)> = Vec::new();
    move |argv: &'static [&'static str]| {
        if let Some((_, output)) = ran.iter().find(|(seen, _)| *seen == argv) {
            return output.clone();
        }
        let output = run(argv);
        ran.push((argv, output.clone()));
        output
    }
}

fn read_output(output: Result<String, String>, read: fn(&str) -> Result<Amount, String>) -> Amount {
    output
        .and_then(|text| read(&text))
        .unwrap_or_else(|reason| Amount::unknown(Why::Failed(reason)))
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

    #[test]
    fn a_survey_runs_each_command_once() {
        let mut runs = Vec::new();
        {
            let mut output = once(|argv| {
                runs.push(argv);
                Ok(argv.join(" "))
            });
            let dry: &'static [&'static str] = &["apt-get", "-s", "autoremove"];
            assert_eq!(output(dry), Ok("apt-get -s autoremove".into()));
            assert_eq!(
                output(&["apt-mark", "showmanual"]),
                Ok("apt-mark showmanual".into())
            );
            assert_eq!(output(dry), Ok("apt-get -s autoremove".into()));
        }
        assert_eq!(runs.len(), 2);
    }

    #[test]
    fn a_command_or_a_reading_that_fails_is_a_failure_and_no_command_is_a_cannot() {
        fn empty(_: &str) -> Result<Amount, String> {
            Ok(sized(Vec::new()))
        }
        fn broken(_: &str) -> Result<Amount, String> {
            Err("unreadable".into())
        }
        let failed = |reason: &str| Size::Unknown(Why::Failed(reason.into()));
        assert_eq!(
            read_output(Err("apt is locked".into()), empty).size,
            failed("apt is locked")
        );
        assert_eq!(
            read_output(Ok(String::new()), broken).size,
            failed("unreadable")
        );
        assert_eq!(read_output(Ok(String::new()), empty).size, Size::Exact(0));
        let mut missing = |_: &'static [&'static str]| Err("no such program".to_string());
        let command = Measure::Command {
            argv: &["qol-cleanup-test-no-such-program"],
            read: empty,
        };
        assert_eq!(
            measure_by(&command, &mut missing).size,
            failed("no such program")
        );
        assert_eq!(
            measure_by(&Measure::Unknown("no dry run"), &mut missing).size,
            Size::Unknown(Why::Cannot("no dry run"))
        );
        let dir = folder("file");
        let file = dir.join("not-a-folder");
        fs::write(&file, b"").expect("write");
        let amount = files(&file.display().to_string(), ".deb");
        let _ = fs::remove_dir_all(&dir);
        assert!(
            matches!(amount.size, Size::Unknown(Why::Failed(_))),
            "{:?}",
            amount.size
        );
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
