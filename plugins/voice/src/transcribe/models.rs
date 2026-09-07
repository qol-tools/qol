use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledModel {
    pub name: String,
    pub path: PathBuf,
}

pub fn models_root() -> Option<PathBuf> {
    qol_config::data_subdir("plugins").map(|path| path.join(crate::PLUGIN_ID).join("models"))
}

pub fn installed_models() -> Vec<InstalledModel> {
    let Some(root) = models_root() else {
        return Vec::new();
    };
    collect_models(&root)
}

fn collect_models(root: &Path) -> Vec<InstalledModel> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut models = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| holds_model_files(&entry.path()))
        .filter_map(|entry| {
            Some(InstalledModel {
                name: entry.file_name().into_string().ok()?,
                path: entry.path(),
            })
        })
        .collect::<Vec<_>>();
    models.sort_by(|left, right| left.name.cmp(&right.name));
    models
}

fn holds_model_files(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries
        .filter_map(|entry| entry.ok())
        .any(|entry| is_weights_file(&entry.file_name().to_string_lossy()))
}

fn is_weights_file(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".onnx") || name.ends_with(".safetensors") || name.ends_with(".bin")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::collect_models;

    #[test]
    fn only_directories_holding_weights_are_offered() {
        let root = TempDir::new().unwrap();
        let usable = root.path().join("parakeet");
        fs::create_dir(&usable).unwrap();
        fs::write(usable.join("encoder.int8.onnx"), b"weights").unwrap();
        fs::write(usable.join("tokens.txt"), b"tokens").unwrap();
        let empty = root.path().join("half-downloaded");
        fs::create_dir(&empty).unwrap();
        fs::write(root.path().join("notes.txt"), b"loose file").unwrap();

        let models = collect_models(root.path());

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "parakeet");
        assert_eq!(models[0].path, usable);
    }

    #[test]
    fn a_missing_models_root_offers_nothing() {
        let root = TempDir::new().unwrap();
        assert!(collect_models(&root.path().join("absent")).is_empty());
    }
}
