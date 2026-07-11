mod platform;

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use tempfile::Builder;

pub fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    atomic_write_inner(path, content, false)
}

pub fn atomic_write_durable(path: &Path, content: &[u8]) -> io::Result<()> {
    atomic_write_inner(path, content, true)
}

fn atomic_write_inner(path: &Path, content: &[u8], durable: bool) -> io::Result<()> {
    let parent = parent_dir(path);
    fs::create_dir_all(parent)?;
    let prefix = temp_prefix(path);
    let mut temp = Builder::new()
        .prefix(&prefix)
        .suffix(".tmp")
        .tempfile_in(parent)?;
    preserve_permissions(path, temp.as_file())?;
    temp.as_file_mut().write_all(content)?;
    if durable {
        temp.as_file().sync_all()?;
    }
    temp.persist(path).map_err(|error| error.error)?;
    if durable {
        platform::sync_parent(parent)?;
    }
    Ok(())
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn temp_prefix(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    format!(".{name}.")
}

fn preserve_permissions(path: &Path, file: &fs::File) -> io::Result<()> {
    let permissions = match fs::metadata(path) {
        Ok(metadata) => metadata.permissions(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    file.set_permissions(permissions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn atomic_write_creates_parents_and_writes_exact_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/state.json");

        atomic_write(&path, b"new content").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new content");
        assert!(temp_files(path.parent().unwrap(), "state.json").is_empty());
    }

    #[test]
    fn atomic_write_replaces_existing_content_without_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, b"old content").unwrap();

        atomic_write(&path, b"new content").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new content");
        assert!(temp_files(dir.path(), "state.json").is_empty());
    }

    #[test]
    fn atomic_write_durable_writes_exact_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/state.json");

        atomic_write_durable(&path, b"durable content").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"durable content");
        assert!(temp_files(path.parent().unwrap(), "state.json").is_empty());
    }

    #[test]
    fn failed_persist_removes_the_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("occupied");
        fs::create_dir(&path).unwrap();

        assert!(atomic_write(&path, b"content").is_err());

        assert!(path.is_dir());
        assert!(temp_files(dir.path(), "occupied").is_empty());
    }

    #[test]
    fn concurrent_writers_never_publish_partial_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("state.json"));
        let payloads = (0..16)
            .map(|index| format!("payload-{index}-{}", "x".repeat(index * 64)))
            .collect::<Vec<_>>();
        let barrier = Arc::new(Barrier::new(payloads.len()));
        let writers = payloads
            .iter()
            .cloned()
            .map(|payload| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    atomic_write(&path, payload.as_bytes()).unwrap();
                })
            })
            .collect::<Vec<_>>();

        for writer in writers {
            writer.join().unwrap();
        }

        let published = fs::read_to_string(path.as_ref()).unwrap();
        assert!(
            payloads.contains(&published),
            "partial content: {published:?}"
        );
        assert!(temp_files(dir.path(), "state.json").is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn replacement_preserves_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        atomic_write(&path, b"new").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    fn temp_files(parent: &Path, target_name: &str) -> Vec<std::path::PathBuf> {
        let prefix = format!(".{target_name}.");
        fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with(&prefix) && name.ends_with(".tmp")
            })
            .map(|entry| entry.path())
            .collect()
    }
}
