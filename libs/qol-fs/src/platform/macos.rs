use std::fs::File;
use std::io;
use std::path::Path;

pub(crate) fn sync_parent(path: &Path) -> io::Result<()> {
    match File::open(path)?.sync_all() {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => Ok(()),
        Err(error) => Err(error),
    }
}
