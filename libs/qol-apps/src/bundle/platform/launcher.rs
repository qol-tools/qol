use std::path::Path;

pub(super) fn exec(path: &Path) -> Vec<String> {
    vec!["open".to_string(), path.display().to_string()]
}
