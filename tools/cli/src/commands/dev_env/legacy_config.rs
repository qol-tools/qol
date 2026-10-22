use std::path::Path;

const LEGACY_FOLDER: &str = "qol";

pub(super) fn move_legacy_config(config_path: &Path) {
    let Some(legacy_root) = dirs::config_dir() else {
        return;
    };
    move_from(&legacy_root.join(LEGACY_FOLDER), config_path);
}

fn move_from(legacy_dir: &Path, config_path: &Path) {
    let Some(target_dir) = config_path.parent() else {
        return;
    };
    let Some(name) = config_path.file_name() else {
        return;
    };
    let legacy = legacy_dir.join(name);
    if config_path.exists() || !legacy.is_file() {
        return;
    }
    if let Err(error) = std::fs::create_dir_all(target_dir) {
        eprintln!(
            "warning: could not create {}: {error}",
            target_dir.display()
        );
        return;
    }
    if let Err(error) = rename_or_copy(&legacy, config_path) {
        eprintln!(
            "warning: could not move {} to {}: {error}",
            legacy.display(),
            config_path.display()
        );
    }
}

fn rename_or_copy(legacy: &Path, target: &Path) -> std::io::Result<()> {
    if std::fs::rename(legacy, target).is_ok() {
        return Ok(());
    }
    std::fs::copy(legacy, target)?;
    std::fs::remove_file(legacy)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "dev-envs.toml";
    const LOCK: &str = ".dev-envs.toml.lock";

    #[test]
    fn moves_the_config_and_leaves_the_lock() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("qol");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join(CONFIG), "image_root = \"/vm\"").unwrap();
        std::fs::write(legacy.join(LOCK), "").unwrap();
        let target = temp.path().join("qol-tray").join(CONFIG);

        move_from(&legacy, &target);

        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "image_root = \"/vm\""
        );
        assert!(!legacy.join(CONFIG).exists());
        assert!(legacy.join(LOCK).exists());
    }

    #[test]
    fn keeps_a_config_that_already_exists() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("qol");
        let target_dir = temp.path().join("qol-tray");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(legacy.join(CONFIG), "old").unwrap();
        std::fs::write(target_dir.join(CONFIG), "new").unwrap();

        move_from(&legacy, &target_dir.join(CONFIG));

        assert_eq!(
            std::fs::read_to_string(target_dir.join(CONFIG)).unwrap(),
            "new"
        );
        assert_eq!(std::fs::read_to_string(legacy.join(CONFIG)).unwrap(), "old");
    }

    #[test]
    fn missing_legacy_config_creates_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("qol-tray").join(CONFIG);

        move_from(&temp.path().join("qol"), &target);

        assert!(!target.exists());
        assert!(!temp.path().join("qol-tray").exists());
    }
}
