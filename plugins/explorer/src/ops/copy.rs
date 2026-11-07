use std::fs::{self, File, Metadata};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::platform;

const CHUNK: usize = 1 << 20;

pub(super) const ALREADY_THERE: &str = "that folder is already there";

pub(super) trait Meter {
    fn advance(&mut self, bytes: u64);
}

pub(super) struct Copied {
    path: PathBuf,
    folder: bool,
    len: u64,
    modified: Option<SystemTime>,
}

impl Copied {
    fn entry(path: &Path, meta: &Metadata) -> Self {
        Self {
            path: path.to_path_buf(),
            folder: meta.is_dir(),
            len: meta.len(),
            modified: meta.modified().ok(),
        }
    }
}

pub(super) fn copy_node(
    src: &Path,
    dst: &Path,
    meter: &mut dyn Meter,
    copied: &mut Vec<Copied>,
) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        platform::copy_link(src, dst)?;
        copied.push(Copied::entry(src, &meta));
        return Ok(());
    }
    if meta.is_file() {
        let read = copy_file(src, dst, meter)?;
        copied.push(Copied::entry(src, &read));
        return Ok(());
    }
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "it is a special system file, not something that can be copied",
        ));
    }
    platform::new_dir(dst)?;
    copied.push(Copied::entry(src, &meta));
    let mut children = fs::read_dir(src)?.collect::<io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        copy_node(&child.path(), &dst.join(child.file_name()), meter, copied)?;
    }
    if let Ok(handle) = platform::open_dir(dst) {
        platform::keep_mode(&handle, &meta)?;
        if let Ok(modified) = meta.modified() {
            let _ = handle.set_modified(modified);
        }
    }
    Ok(())
}

pub(super) fn copy_file(src: &Path, dst: &Path, meter: &mut dyn Meter) -> io::Result<Metadata> {
    let outcome = write_verified(src, dst, meter);
    if outcome.is_err() {
        let _ = fs::remove_file(dst);
    }
    outcome
}

fn write_verified(src: &Path, dst: &Path, meter: &mut dyn Meter) -> io::Result<Metadata> {
    let mut reader = File::open(src)?;
    let meta = reader.metadata()?;
    let mut writer = platform::new_file(dst)?;
    let mut buffer = vec![0u8; CHUNK];
    let mut written = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        written += read as u64;
        meter.advance(read as u64);
    }
    writer.flush()?;
    if let Ok(modified) = meta.modified() {
        writer.set_modified(modified)?;
    }
    platform::keep_mode(&writer, &meta)?;
    if written != meta.len() || writer.metadata()?.len() != written {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the copy came out a different size",
        ));
    }
    Ok(meta)
}

pub(super) fn same_object(a: &Path, b: &Path) -> bool {
    let identity = |path: &Path| {
        let meta = fs::metadata(path).ok()?;
        qol_fs::file_identity(path, &meta)
    };
    match (identity(a), identity(b)) {
        (Some(left), Some(right)) => left.device == right.device && left.index == right.index,
        _ => matches!(
            (fs::canonicalize(a), fs::canonicalize(b)),
            (Ok(left), Ok(right)) if left == right
        ),
    }
}

pub(super) fn same_entry(a: &Path, b: &Path) -> bool {
    let plain =
        |path: &Path| fs::symlink_metadata(path).is_ok_and(|meta| !meta.file_type().is_symlink());
    plain(a) && plain(b) && same_object(a, b)
}

pub(super) fn same_link(a: &Path, b: &Path) -> bool {
    matches!((fs::read_link(a), fs::read_link(b)), (Ok(left), Ok(right)) if left == right)
}

pub(super) fn same_content(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) else {
        return false;
    };
    if !ma.is_file() || !mb.is_file() || ma.len() != mb.len() {
        return false;
    }
    let (Ok(mut fa), Ok(mut fb)) = (File::open(a), File::open(b)) else {
        return false;
    };
    let mut left = vec![0u8; CHUNK];
    let mut right = vec![0u8; CHUNK];
    loop {
        let Ok(read) = fa.read(&mut left) else {
            return false;
        };
        if read == 0 {
            return true;
        }
        if fb.read_exact(&mut right[..read]).is_err() || left[..read] != right[..read] {
            return false;
        }
    }
}

pub(super) fn remove_all(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

pub(super) fn remove_copied(copied: &[Copied]) -> io::Result<()> {
    for entry in copied.iter().rev() {
        if entry.folder {
            fs::remove_dir(&entry.path)?;
            continue;
        }
        let meta = fs::symlink_metadata(&entry.path)?;
        if meta.len() != entry.len || meta.modified().ok() != entry.modified {
            return Err(io::Error::other(
                "it changed while it was being copied, so it was left where it is",
            ));
        }
        fs::remove_file(&entry.path)?;
    }
    Ok(())
}
