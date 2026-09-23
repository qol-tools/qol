use std::fs::File;
use std::io;
use std::path::Path;

pub(crate) fn set_private(_: &File) -> io::Result<()> {
    Ok(())
}

pub(crate) fn set_private_dir(_: &Path) -> io::Result<()> {
    Ok(())
}

pub(crate) fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

pub(crate) fn prepare_file_removal(_: &Path) -> io::Result<()> {
    Ok(())
}

pub(crate) fn set_mode(_: &File, _: u32) -> io::Result<()> {
    Ok(())
}
