use std::fs::{self, File, Metadata, OpenOptions};
use std::io;
use std::path::Path;
use std::time::SystemTime;

pub(crate) fn new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

pub(crate) fn new_dir(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

pub(crate) fn open_dir(path: &Path) -> io::Result<File> {
    File::open(path)
}

pub(crate) fn keep_mode(file: &File, source: &Metadata) -> io::Result<()> {
    file.set_permissions(source.permissions())
}

pub(crate) fn changed_at(_meta: &Metadata) -> Option<(i64, i64)> {
    None
}

pub(crate) fn born_at(_meta: &Metadata) -> Option<SystemTime> {
    None
}
