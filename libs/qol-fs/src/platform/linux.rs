use std::fs::File;
use std::io;
use std::path::Path;

pub(crate) fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}
