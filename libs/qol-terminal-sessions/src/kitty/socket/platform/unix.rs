use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

pub(super) fn exchange(path: &Path, request: &[u8], terminator: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.write_all(request)?;
    stream.flush()?;

    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    while !response.ends_with(terminator) {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        response.extend_from_slice(&chunk[..read]);
    }
    Ok(response)
}

pub(super) fn instance_id(path: &Path) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    metadata
        .file_type()
        .is_socket()
        .then(|| format!("k{:x}_{:x}", metadata.dev(), metadata.ino()))
}

pub(super) fn discover_sibling_paths(current: &Path) -> Vec<PathBuf> {
    let Some(current_metadata) = std::fs::symlink_metadata(current).ok() else {
        return Vec::new();
    };
    if !current_metadata.file_type().is_socket() {
        return Vec::new();
    }
    let Some(parent) = current.parent() else {
        return vec![current.to_owned()];
    };
    let Some(current_name) = current.file_name().and_then(|name| name.to_str()) else {
        return vec![current.to_owned()];
    };
    if !is_default_kitty_socket_name(current_name) {
        return vec![current.to_owned()];
    }
    let Ok(entries) = std::fs::read_dir(parent) else {
        return vec![current.to_owned()];
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(is_default_kitty_socket_name)
        })
        .filter_map(|entry| {
            if !entry.file_type().ok()?.is_socket() {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            (metadata.uid() == current_metadata.uid()).then(|| entry.path())
        })
        .collect()
}

fn is_default_kitty_socket_name(name: &str) -> bool {
    name.strip_prefix("kitty-")
        .and_then(|suffix| suffix.strip_suffix(".sock").or(Some(suffix)))
        .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;

    use super::{discover_sibling_paths, instance_id, is_default_kitty_socket_name};

    #[test]
    fn default_socket_names_are_narrowly_recognized() {
        for name in ["kitty-12", "kitty-12.sock"] {
            assert!(is_default_kitty_socket_name(name), "{name}");
        }
        for name in ["kitty-", "kitty-main", "other-12", "kitty-12.tmp"] {
            assert!(!is_default_kitty_socket_name(name), "{name}");
        }
    }

    #[test]
    fn sibling_discovery_keeps_owned_kitty_sockets_and_rejects_lookalikes() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("kitty-11");
        let sibling = dir.path().join("kitty-12");
        let unrelated = dir.path().join("other-13");
        let alias = dir.path().join("kitty-14");
        let _current_listener = UnixListener::bind(&current).unwrap();
        let _sibling_listener = UnixListener::bind(&sibling).unwrap();
        let _unrelated_listener = UnixListener::bind(&unrelated).unwrap();
        std::os::unix::fs::symlink(&sibling, &alias).unwrap();

        let mut paths = discover_sibling_paths(&current);
        paths.sort();

        assert_eq!(paths, [current.clone(), sibling.clone()]);
        assert_ne!(instance_id(&current), instance_id(&sibling));
        assert!(instance_id(&alias).is_none());
    }
}
