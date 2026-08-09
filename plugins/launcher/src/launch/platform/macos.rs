use std::io;
use std::path::Path;

pub(crate) fn launch_app(path: &Path, _exec: &[String]) -> io::Result<()> {
    super::super::open_path(path)
}
