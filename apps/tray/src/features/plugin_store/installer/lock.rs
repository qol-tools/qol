use std::path::Path;
use std::time::Duration;

pub(super) fn open_lock_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
}

pub(super) fn stale_lockfile(path: &Path, max_age: Duration) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return lockfile_too_old(path, max_age);
    };
    let Some(raw_pid) = content.split_whitespace().next() else {
        return lockfile_too_old(path, max_age);
    };
    let Ok(pid) = raw_pid.parse::<u32>() else {
        return lockfile_too_old(path, max_age);
    };

    if let Some(alive) = super::super::platform::lock_owner_alive(pid) {
        return !alive;
    }
    lockfile_too_old(path, max_age)
}

fn lockfile_too_old(path: &Path, max_age: Duration) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    modified.elapsed().is_ok_and(|age| age > max_age)
}
