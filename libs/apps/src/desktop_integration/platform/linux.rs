use std::path::Path;

use super::RevealPlan;

pub(super) fn reveal_plan(path: &Path) -> RevealPlan {
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    target.to_path_buf().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_opens_a_files_parent_directory() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("capture.png");
        std::fs::write(&file, []).unwrap();

        let RevealPlan::Open(target) = reveal_plan(&file) else {
            panic!("Linux reveal must open a directory");
        };
        assert_eq!(target, temp.path());
    }

    #[test]
    fn reveal_opens_a_directory_itself() {
        let temp = tempfile::tempdir().unwrap();
        let RevealPlan::Open(target) = reveal_plan(temp.path()) else {
            panic!("Linux reveal must open a directory");
        };
        assert_eq!(target, temp.path());
    }
}
