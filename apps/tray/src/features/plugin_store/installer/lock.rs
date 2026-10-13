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

pub(super) fn owner_identity() -> Option<&'static str> {
    static IDENTITY: OnceLock<Option<String>> = OnceLock::new();
    IDENTITY
        .get_or_init(|| qol_process::process_identity(std::process::id()).ok())
        .as_deref()
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
    let _plugin_id = fields.next();
    let owner_instance = fields.next();
    let recorded_identity = fields.next();
    if pid == std::process::id() {
        return owner_instance != Some(instance());
    }

    if let Some(alive) = super::super::platform::lock_owner_alive(pid) {
        return !alive || recorded_identity.is_some_and(|expected| owner_replaced(pid, expected));
    }
    lockfile_too_old(path, max_age)
}

fn owner_replaced(pid: u32, expected: &str) -> bool {
    !qol_process::process_identity_matches(pid, expected)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owner_whose_identity_cannot_be_read_is_replaced() {
        assert!(owner_replaced(u32::MAX - 1, "recorded-identity"));
    }

    #[test]
    fn an_owner_is_replaced_only_when_its_identity_differs() {
        let pid = std::process::id();
        let identity = owner_identity().expect("own identity");

        assert!(!owner_replaced(pid, identity));
        assert!(owner_replaced(pid, "another-process"));
    }
}
