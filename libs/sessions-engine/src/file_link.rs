use std::fs;
use std::io;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placed {
    Shared,
    Linked,
    Copied,
}

pub(crate) fn is_same_file(left: &Path, right: &Path) -> bool {
    let identity = |path: &Path| {
        let metadata = fs::symlink_metadata(path).ok()?;
        if !metadata.file_type().is_file() {
            return None;
        }
        qol_fs::file_identity(path, &metadata)
    };
    match (identity(left), identity(right)) {
        (Some(left), Some(right)) => left.device == right.device && left.index == right.index,
        _ => false,
    }
}

pub(crate) fn link(source: &Path, destination: &Path, temporary: &Path) -> io::Result<()> {
    if is_same_file(source, destination) {
        return Ok(());
    }
    let placed = fs::hard_link(source, temporary).and_then(|()| fs::rename(temporary, destination));
    if placed.is_err() {
        let _ = fs::remove_file(temporary);
    }
    placed
}

pub(crate) fn link_or_copy(
    source: &Path,
    destination: &Path,
    temporary: &Path,
) -> io::Result<Placed> {
    if is_same_file(source, destination) {
        return Ok(Placed::Shared);
    }
    if link(source, destination, temporary).is_ok() {
        return Ok(Placed::Linked);
    }
    let copied = fs::copy(source, temporary).and_then(|_| fs::rename(temporary, destination));
    if let Err(error) = copied {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    Ok(Placed::Copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linked_file_is_the_same_file_and_grows_with_its_source() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.jsonl");
        let destination = root.path().join("destination.jsonl");
        let temporary = root.path().join("destination.tmp");
        fs::write(&source, "one\n").unwrap();
        fs::write(&destination, "stale\n").unwrap();
        assert!(!is_same_file(&source, &destination));

        let placed = link_or_copy(&source, &destination, &temporary).unwrap();

        assert_eq!(placed, Placed::Linked);
        assert!(is_same_file(&source, &destination));
        assert!(!temporary.exists());
        let mut grown = fs::OpenOptions::new().append(true).open(&source).unwrap();
        io::Write::write_all(&mut grown, b"two\n").unwrap();
        assert_eq!(fs::read_to_string(&destination).unwrap(), "one\ntwo\n");
        assert_eq!(
            link_or_copy(&source, &destination, &temporary).unwrap(),
            Placed::Shared
        );
    }

    #[test]
    fn linking_a_file_onto_itself_leaves_no_temporary_link() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.jsonl");
        let destination = root.path().join("destination.jsonl");
        let temporary = root.path().join("destination.tmp");
        fs::write(&source, "one\n").unwrap();
        link(&source, &destination, &temporary).unwrap();

        link(&source, &destination, &temporary).unwrap();

        assert!(is_same_file(&source, &destination));
        assert!(!temporary.exists());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }

    #[test]
    fn a_removed_source_leaves_the_linked_file_whole() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.jsonl");
        let destination = root.path().join("destination.jsonl");
        fs::write(&source, "kept\n").unwrap();
        link_or_copy(&source, &destination, &root.path().join("tmp")).unwrap();

        fs::remove_file(&source).unwrap();

        assert_eq!(fs::read_to_string(&destination).unwrap(), "kept\n");
    }

    #[test]
    fn a_source_that_cannot_be_linked_is_copied_and_a_missing_one_fails_clean() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.jsonl");
        let destination = root.path().join("destination.jsonl");
        let temporary = root.path().join("destination.tmp");
        fs::write(&source, "one\n").unwrap();
        fs::write(&temporary, "in the way").unwrap();

        assert!(link(&source, &destination, &temporary).is_err());
        assert!(!temporary.exists());
        assert!(!destination.exists());

        fs::write(&temporary, "in the way").unwrap();
        assert_eq!(
            link_or_copy(&source, &destination, &temporary).unwrap(),
            Placed::Copied
        );
        assert_eq!(fs::read_to_string(&destination).unwrap(), "one\n");
        assert!(!is_same_file(&source, &destination));
        assert!(!temporary.exists());
        fs::remove_file(&destination).unwrap();

        let missing = root.path().join("missing.jsonl");
        assert!(link_or_copy(&missing, &destination, &temporary).is_err());
        assert!(!temporary.exists());
        assert!(!destination.exists());
        assert!(!is_same_file(&missing, &destination));
    }
}
