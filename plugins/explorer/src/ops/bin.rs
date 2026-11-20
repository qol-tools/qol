use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::platform;
use super::print::Print;
use super::simple::plain;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub item: PathBuf,
    pub sidecar: Option<PathBuf>,
    pub before: Print,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinError {
    NoTrash(String),
    Gone,
    Changed,
    Taken,
    Failed(String),
}

impl BinError {
    pub fn failed(error: &io::Error) -> Self {
        Self::Failed(plain(error))
    }

    pub fn reason(&self) -> String {
        match self {
            Self::NoTrash(reason) | Self::Failed(reason) => reason.clone(),
            Self::Gone => "it is no longer there".into(),
            Self::Changed => "something else is in its place in the trash".into(),
            Self::Taken => "something with that name is already there".into(),
        }
    }
}

pub trait Bin: Send + Sync {
    fn trash(&self, path: &Path) -> Result<Option<Held>, BinError>;
    fn restore(&self, held: &Held, to: &Path) -> Result<(), BinError>;
}

pub fn system() -> Arc<dyn Bin> {
    platform::system_bin()
}

pub fn restore_exact(held: &Held, to: &Path) -> Result<(), BinError> {
    let now = match Print::read(&held.item) {
        Ok(now) => now,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(BinError::Gone),
        Err(error) => return Err(BinError::failed(&error)),
    };
    if !held.before.same_object(&now) {
        return Err(BinError::Changed);
    }
    match qol_fs::rename_noreplace(&held.item, to) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(BinError::Taken),
        Err(error) => return Err(BinError::failed(&error)),
    }
    if let Some(sidecar) = &held.sidecar {
        if let Err(error) = fs::remove_file(sidecar) {
            if error.kind() != io::ErrorKind::NotFound {
                log::warn!("could not remove {}: {error}", sidecar.display());
            }
        }
    }
    Ok(())
}

pub struct FolderBin {
    root: PathBuf,
    serial: AtomicU64,
}

impl FolderBin {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            serial: AtomicU64::new(0),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Bin for FolderBin {
    fn trash(&self, path: &Path) -> Result<Option<Held>, BinError> {
        let before = Print::read(path).map_err(|error| BinError::failed(&error))?;
        let name = path
            .file_name()
            .ok_or_else(|| BinError::Failed("it has no name".into()))?;
        fs::create_dir_all(&self.root).map_err(|error| BinError::failed(&error))?;
        loop {
            let serial = self.serial.fetch_add(1, Ordering::Relaxed);
            let item = self
                .root
                .join(format!("{serial}-{}", name.to_string_lossy()));
            match qol_fs::rename_noreplace(path, &item) {
                Ok(()) => {
                    return Ok(Some(Held {
                        item,
                        sidecar: None,
                        before,
                    }))
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(BinError::failed(&error)),
            }
        }
    }

    fn restore(&self, held: &Held, to: &Path) -> Result<(), BinError> {
        restore_exact(held, to)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use tempfile::TempDir;

    use super::{Bin, BinError, FolderBin};
    use crate::ops::print::Print;

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(1000))
            .unwrap();
    }

    fn replace(path: &Path, text: &str) {
        let swap = path.with_file_name(".swap");
        write(&swap, text);
        fs::rename(&swap, path).unwrap();
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn setup() -> (TempDir, FolderBin) {
        let dir = tempfile::tempdir().unwrap();
        let bin = FolderBin::new(dir.path().join("bin"));
        (dir, bin)
    }

    fn work(dir: &TempDir, relative: &str) -> PathBuf {
        dir.path().join("work").join(relative)
    }

    #[test]
    fn trash_moves_the_item_into_the_root_under_a_new_name() {
        let (dir, bin) = setup();
        let path = work(&dir, "a.txt");
        write(&path, "text");
        let before = Print::read(&path).unwrap();
        let held = bin.trash(&path).unwrap().unwrap();
        assert!(!path.exists());
        assert_eq!(held.item.parent(), Some(bin.root()));
        assert_ne!(held.item.file_name(), path.file_name());
        assert_eq!(read(&held.item), "text");
        assert_eq!(held.sidecar, None);
        assert_eq!(held.before, before);
        assert!(held.before.unchanged(&Print::read(&held.item).unwrap()));
    }

    #[test]
    fn trash_makes_the_root_when_it_is_missing() {
        let (dir, bin) = setup();
        let path = work(&dir, "a.txt");
        write(&path, "text");
        assert!(!bin.root().exists());
        bin.trash(&path).unwrap();
        assert!(bin.root().is_dir());
    }

    #[test]
    fn two_items_with_one_name_never_collide() {
        let (dir, bin) = setup();
        let paths = [
            work(&dir, "one/a.txt"),
            work(&dir, "two/a.txt"),
            work(&dir, "three/a.txt"),
        ];
        let mut held = Vec::new();
        for (number, path) in paths.iter().enumerate() {
            write(path, &format!("text {number}"));
            held.push(bin.trash(path).unwrap().unwrap());
        }
        for (number, held) in held.iter().enumerate() {
            assert_eq!(read(&held.item), format!("text {number}"));
        }
        let mut items: Vec<&PathBuf> = held.iter().map(|held| &held.item).collect();
        items.sort();
        items.dedup();
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn trash_never_replaces_a_name_already_in_the_root() {
        let (dir, bin) = setup();
        let squatter = bin.root().join("0-a.txt");
        write(&squatter, "squatter");
        let path = work(&dir, "a.txt");
        write(&path, "text");
        let held = bin.trash(&path).unwrap().unwrap();
        assert_ne!(held.item, squatter);
        assert_eq!(read(&squatter), "squatter");
        assert_eq!(read(&held.item), "text");
    }

    #[test]
    fn trash_of_a_missing_path_fails_and_changes_nothing() {
        let (dir, bin) = setup();
        let result = bin.trash(&work(&dir, "missing.txt"));
        assert!(matches!(result, Err(BinError::Failed(_))), "{result:?}");
        assert!(!bin.root().exists());
    }

    #[test]
    fn restore_puts_the_item_back() {
        let (dir, bin) = setup();
        let path = work(&dir, "a.txt");
        write(&path, "text");
        let held = bin.trash(&path).unwrap().unwrap();
        assert_eq!(bin.restore(&held, &path), Ok(()));
        assert_eq!(read(&path), "text");
        assert!(!held.item.exists());
    }

    #[test]
    fn restore_puts_a_folder_back_with_what_it_holds() {
        let (dir, bin) = setup();
        let path = work(&dir, "folder");
        write(&path.join("inner/b.txt"), "inner");
        let held = bin.trash(&path).unwrap().unwrap();
        assert!(!path.exists());
        assert_eq!(bin.restore(&held, &path), Ok(()));
        assert_eq!(read(&path.join("inner/b.txt")), "inner");
    }

    #[test]
    fn restore_onto_a_taken_place_is_taken_and_leaves_both() {
        let (dir, bin) = setup();
        let path = work(&dir, "a.txt");
        write(&path, "old");
        let held = bin.trash(&path).unwrap().unwrap();
        write(&path, "new");
        assert_eq!(bin.restore(&held, &path), Err(BinError::Taken));
        assert_eq!(read(&path), "new");
        assert_eq!(read(&held.item), "old");
    }

    #[test]
    fn a_swapped_bin_item_is_changed_and_stays_where_it_is() {
        let (dir, bin) = setup();
        let path = work(&dir, "a.txt");
        write(&path, "old");
        let held = bin.trash(&path).unwrap().unwrap();
        replace(&held.item, "swapped");
        assert_eq!(bin.restore(&held, &path), Err(BinError::Changed));
        assert!(!path.exists());
        assert_eq!(read(&held.item), "swapped");
    }

    #[test]
    fn a_removed_bin_item_is_gone() {
        let (dir, bin) = setup();
        let path = work(&dir, "a.txt");
        write(&path, "old");
        let held = bin.trash(&path).unwrap().unwrap();
        fs::remove_file(&held.item).unwrap();
        assert_eq!(bin.restore(&held, &path), Err(BinError::Gone));
        assert!(!path.exists());
    }

    #[test]
    fn restore_removes_the_sidecar_and_tolerates_one_that_is_gone() {
        let (dir, bin) = setup();
        let path = work(&dir, "a.txt");
        write(&path, "text");
        let mut held = bin.trash(&path).unwrap().unwrap();
        let sidecar = bin.root().join("a.trashinfo");
        write(&sidecar, "info");
        held.sidecar = Some(sidecar.clone());
        assert_eq!(bin.restore(&held, &path), Ok(()));
        assert!(!sidecar.exists());

        let again = work(&dir, "b.txt");
        write(&again, "text");
        let mut held = bin.trash(&again).unwrap().unwrap();
        held.sidecar = Some(bin.root().join("never-there.trashinfo"));
        assert_eq!(bin.restore(&held, &again), Ok(()));
        assert_eq!(read(&again), "text");
    }
}
