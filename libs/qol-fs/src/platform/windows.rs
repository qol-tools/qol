use std::io;
use std::path::Path;

pub(crate) fn sync_parent(_: &Path) -> io::Result<()> {
    Ok(())
}
