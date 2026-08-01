use std::path::{Path, PathBuf};

pub(super) fn workspace_root() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)?;
    root.join("Cargo.toml")
        .is_file()
        .then(|| root.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_resolves_to_dir_with_cargo_toml() {
        let root = workspace_root().expect("workspace root resolves in-tree");
        assert!(
            root.join("Cargo.toml").is_file(),
            "root: {}",
            root.display()
        );
    }
}
