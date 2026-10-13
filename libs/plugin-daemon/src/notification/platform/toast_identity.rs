use std::io;
use std::path::Path;

pub(super) const RECORD_FILE: &str = "toast-identity-created";

pub(super) trait IdentityKey {
    fn exists(&self) -> io::Result<bool>;
    fn create(&self) -> io::Result<()>;
    fn delete(&self) -> io::Result<()>;
}

pub(super) fn claim(key: &impl IdentityKey, record: &Path) -> io::Result<()> {
    if key.exists()? {
        return Ok(());
    }
    if let Some(parent) = record.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(record, b"")?;
    key.create()
}

pub(super) fn release(key: &impl IdentityKey, record: &Path) -> io::Result<()> {
    if !record.exists() {
        return Ok(());
    }
    key.delete()?;
    remove_record(record)
}

pub(super) fn install(key: &impl IdentityKey, record: &Path) -> io::Result<()> {
    key.create()?;
    remove_record(record)
}

pub(super) fn uninstall(key: &impl IdentityKey, record: &Path) -> io::Result<()> {
    key.delete()?;
    remove_record(record)
}

fn remove_record(record: &Path) -> io::Result<()> {
    match std::fs::remove_file(record) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct FakeKey {
        present: Cell<bool>,
        creates: Cell<usize>,
        deletes: Cell<usize>,
    }

    impl FakeKey {
        fn present() -> Self {
            let key = Self::default();
            key.present.set(true);
            key
        }
    }

    impl IdentityKey for FakeKey {
        fn exists(&self) -> io::Result<bool> {
            Ok(self.present.get())
        }

        fn create(&self) -> io::Result<()> {
            self.present.set(true);
            self.creates.set(self.creates.get() + 1);
            Ok(())
        }

        fn delete(&self) -> io::Result<()> {
            self.present.set(false);
            self.deletes.set(self.deletes.get() + 1);
            Ok(())
        }
    }

    fn record_in(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("qol-tray").join(RECORD_FILE)
    }

    #[test]
    fn a_key_qol_did_not_create_is_never_recorded_or_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let record = record_in(&dir);
        let key = FakeKey::present();

        claim(&key, &record).unwrap();
        release(&key, &record).unwrap();

        assert!(key.present.get());
        assert_eq!((key.creates.get(), key.deletes.get()), (0, 0));
        assert!(!record.exists());
    }

    #[test]
    fn a_key_qol_created_is_recorded_and_released_once() {
        let dir = tempfile::tempdir().unwrap();
        let record = record_in(&dir);
        let key = FakeKey::default();

        claim(&key, &record).unwrap();
        claim(&key, &record).unwrap();
        assert!(record.exists());
        assert_eq!(key.creates.get(), 1);

        release(&key, &record).unwrap();
        release(&key, &record).unwrap();
        assert!(!key.present.get());
        assert!(!record.exists());
        assert_eq!(key.deletes.get(), 1);
    }

    #[test]
    fn a_record_left_by_a_crash_releases_the_key_at_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let record = record_in(&dir);
        let crashed = FakeKey::default();
        claim(&crashed, &record).unwrap();

        let next_start = FakeKey::present();
        release(&next_start, &record).unwrap();

        assert!(!next_start.present.get());
        assert!(!record.exists());
    }

    #[test]
    fn the_installer_takes_over_a_portable_key_and_the_uninstaller_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let record = record_in(&dir);
        let key = FakeKey::default();
        claim(&key, &record).unwrap();

        install(&key, &record).unwrap();
        assert!(!record.exists());
        release(&key, &record).unwrap();
        assert!(key.present.get(), "an installed key outlives tray exit");

        uninstall(&key, &record).unwrap();
        assert!(!key.present.get());
        uninstall(&key, &record).unwrap();
    }
}
