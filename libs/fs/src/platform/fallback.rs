use std::fs::File;
use std::io;
use std::path::Path;

pub(crate) fn set_private(_: &File) -> io::Result<()> {
    Ok(())
}

pub(crate) fn set_private_dir(_: &Path) -> io::Result<()> {
    Ok(())
}

pub(crate) fn ensure_private_file(_: &Path, _: &std::fs::Metadata) -> io::Result<()> {
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

pub fn file_identity(_: &Path, _: &std::fs::Metadata) -> Option<crate::FileIdentity> {
    None
}

pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    super::rename_check::rename_if_absent(from, to)
}
