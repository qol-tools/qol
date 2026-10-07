use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) fn instance() -> &'static str {
    static INSTANCE: OnceLock<String> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        format!("{:x}", started.as_nanos())
    })
}

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
    if !owner_line_complete(&content) {
        return lockfile_too_old(path, max_age);
    }
    let mut fields = content.split_whitespace();
    let Some(raw_pid) = fields.next() else {
        return lockfile_too_old(path, max_age);
    };
    let Ok(pid) = raw_pid.parse::<u32>() else {
        return lockfile_too_old(path, max_age);
    };
    if pid == std::process::id() {
        return fields.nth(1) != Some(instance());
    }

    if let Some(alive) = super::super::platform::lock_owner_alive(pid) {
        return !alive;
    }
    lockfile_too_old(path, max_age)
}

fn owner_line_complete(content: &str) -> bool {
    content.ends_with('\n')
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
