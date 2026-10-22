use std::ffi::OsStr;
use std::path::Path;

pub(crate) fn move_legacy_store(frecency_path: &Path) {
    let Some(legacy_dir) = dirs::cache_dir() else {
        return;
    };
    move_from(&legacy_dir, frecency_path);
}

fn move_from(legacy_dir: &Path, frecency_path: &Path) {
    let Some(target_dir) = frecency_path.parent() else {
        return;
    };
    let Some(store_name) = frecency_path.file_name() else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(target_dir) {
        log::warn!(
            "legacy store: could not create {}: {error}",
            target_dir.display()
        );
        return;
    }
    let boosts_name = OsStr::new(qol_conventions::launcher::BOOSTS_FILE_NAME);
    for name in [store_name, boosts_name] {
        move_file(&legacy_dir.join(name), &target_dir.join(name));
    }
}

fn move_file(legacy: &Path, target: &Path) {
    if target.exists() || !legacy.is_file() {
        return;
    }
    if let Err(error) = qol_fs::move_file(legacy, target) {
        log::warn!(
            "legacy store: could not move {} to {}: {error}",
            legacy.display(),
            target.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORE: &str = "qol-launcher-frequency.json";

    fn boosts() -> &'static str {
        qol_conventions::launcher::BOOSTS_FILE_NAME
    }

    #[test]
    fn moves_both_files_into_a_new_folder() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("cache");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join(STORE), "store").unwrap();
        std::fs::write(legacy.join(boosts()), "boosts").unwrap();
        let target = temp.path().join("data/qol-tray");

        move_from(&legacy, &target.join(STORE));

        assert_eq!(
            std::fs::read_to_string(target.join(STORE)).unwrap(),
            "store"
        );
        assert_eq!(
            std::fs::read_to_string(target.join(boosts())).unwrap(),
            "boosts"
        );
        assert!(!legacy.join(STORE).exists());
        assert!(!legacy.join(boosts()).exists());
    }

    #[test]
    fn keeps_a_target_that_already_exists() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("cache");
        let target = temp.path().join("data");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(legacy.join(STORE), "old").unwrap();
        std::fs::write(target.join(STORE), "new").unwrap();

        move_from(&legacy, &target.join(STORE));

        assert_eq!(std::fs::read_to_string(target.join(STORE)).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(legacy.join(STORE)).unwrap(), "old");
    }

    #[test]
    fn missing_legacy_files_leave_the_target_empty() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("cache");
        let target = temp.path().join("data");

        move_from(&legacy, &target.join(STORE));

        assert!(target.is_dir());
        assert!(!target.join(STORE).exists());
        assert!(!target.join(boosts()).exists());
    }
}
