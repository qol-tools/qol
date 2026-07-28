use std::fs::File;
use std::io;
use std::path::Path;

pub(crate) fn set_private(_: &File) -> io::Result<()> {
    Ok(())
}

pub(crate) fn set_private_dir(_: &Path) -> io::Result<()> {
    Ok(())
}

pub(crate) fn sync_parent(_: &Path) -> io::Result<()> {
    Ok(())
}
