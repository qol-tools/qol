use std::path::Path;

pub(crate) fn launch_app(path: &Path, _exec: &[String]) -> bool {
    super::super::open_path(path)
}
