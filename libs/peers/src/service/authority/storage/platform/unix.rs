use std::{
    fs::{self, DirBuilder, File, Metadata, OpenOptions, TryLockError},
    io,
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

use zeroize::Zeroizing;

use crate::service::authority::{state::MAX_SNAPSHOT_BYTES, AuthorityError};

pub(in crate::service::authority) struct Store {
    canonical_root: PathBuf,
    directory: File,
    lock: File,
    snapshot: Option<File>,
    #[cfg(test)]
    fault: Option<super::CommitFault>,
}

impl Store {
    pub fn create(root: &Path) -> Result<Self, AuthorityError> {
        DirBuilder::new()
            .mode(0o700)
            .create(root)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    return AuthorityError::AlreadyExists;
                }
                io_error(error)
            })?;
        let store = Self::acquire(root, true)?;
        let parent = store
            .canonical_root
            .parent()
            .ok_or(AuthorityError::UnsafeStore)?;
        let parent = open_directory(parent)?;
        store.directory.sync_all().map_err(io_error)?;
        parent.sync_all().map_err(io_error)?;
        Ok(store)
    }

    pub fn open(root: &Path) -> Result<Self, AuthorityError> {
        Self::acquire(root, false)
    }

    fn acquire(root: &Path, create: bool) -> Result<Self, AuthorityError> {
        let root: PathBuf = root.components().collect();
        let metadata = fs::symlink_metadata(&root).map_err(io_error)?;
        private_directory(&metadata)?;
        let canonical_root = root.canonicalize().map_err(io_error)?;
        let directory = open_directory(&canonical_root)?;
        same_inode(&metadata, &directory.metadata().map_err(io_error)?)?;
        let anchored = anchored_root(&directory)?;
        same_inode(&metadata, &fs::metadata(&anchored).map_err(io_error)?)?;
        let lock_path = anchored.join("writer.lock");
        let lock = open_regular(&lock_path, create)?;
        lock.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => AuthorityError::WriterBusy,
            TryLockError::Error(_) => AuthorityError::Storage,
        })?;
        check_file(&lock_path, &lock)?;
        let snapshot = if create {
            None
        } else {
            Some(open_regular(&anchored.join("state.json"), false)?)
        };
        let store = Self {
            canonical_root,
            directory,
            lock,
            snapshot,
            #[cfg(test)]
            fault: None,
        };
        store.validate()?;
        Ok(store)
    }

    pub fn read(&self) -> Result<Zeroizing<Vec<u8>>, AuthorityError> {
        self.validate()?;
        let file = self.snapshot.as_ref().ok_or(AuthorityError::MissingStore)?;
        let length = file.metadata().map_err(io_error)?.len();
        if length > MAX_SNAPSHOT_BYTES as u64 {
            return Err(AuthorityError::Capacity);
        }
        let mut bytes = Zeroizing::new(vec![0; length as usize]);
        file.read_exact_at(&mut bytes, 0).map_err(io_error)?;
        let mut extra = Zeroizing::new([0_u8; 1]);
        if file.read_at(extra.as_mut(), length).map_err(io_error)? != 0 {
            return Err(AuthorityError::Capacity);
        }
        self.validate()?;
        if file.metadata().map_err(io_error)?.len() != length {
            return Err(AuthorityError::UnsafeStore);
        }
        Ok(bytes)
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), AuthorityError> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(AuthorityError::Capacity);
        }
        self.validate()?;
        #[cfg(test)]
        let fault = self.fault.take();
        #[cfg(test)]
        if matches!(fault, Some(super::CommitFault::BeforeReplace)) {
            return Err(AuthorityError::Storage);
        }
        let path = anchored_root(&self.directory)?.join("state.json");
        qol_fs::atomic_write_private(&path, bytes).map_err(io_error)?;
        #[cfg(test)]
        if matches!(fault, Some(super::CommitFault::AfterReplace)) {
            return Err(AuthorityError::Storage);
        }
        self.snapshot = Some(open_regular(&path, false)?);
        self.validate()?;
        self.directory.sync_all().map_err(io_error)
    }

    fn validate(&self) -> Result<(), AuthorityError> {
        let current = fs::symlink_metadata(&self.canonical_root).map_err(io_error)?;
        private_directory(&current)?;
        let held = self.directory.metadata().map_err(io_error)?;
        private_directory(&held)?;
        same_inode(&current, &held)?;
        let anchored = anchored_root(&self.directory)?;
        check_file(&anchored.join("writer.lock"), &self.lock)?;
        let path = anchored.join("state.json");
        if let Some(snapshot) = &self.snapshot {
            return check_file(&path, snapshot);
        }
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(error)),
            Ok(_) => Err(AuthorityError::UnsafeStore),
        }
    }

    #[cfg(test)]
    pub fn fail_next(&mut self, fault: super::CommitFault) {
        self.fault = Some(fault);
    }
}

#[cfg(target_os = "linux")]
fn anchored_root(directory: &File) -> Result<PathBuf, AuthorityError> {
    Ok(PathBuf::from(format!(
        "/proc/self/fd/{}",
        directory.as_raw_fd()
    )))
}

#[cfg(target_os = "macos")]
fn anchored_root(directory: &File) -> Result<PathBuf, AuthorityError> {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let mut buffer = [0_u8; libc::PATH_MAX as usize];
    if unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) } == -1 {
        return Err(io_error(io::Error::last_os_error()));
    }
    let length = buffer
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(AuthorityError::Storage)?;
    Ok(PathBuf::from(OsStr::from_bytes(&buffer[..length])))
}

fn open_directory(path: &Path) -> Result<File, AuthorityError> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(io_error)
}

fn open_regular(path: &Path, create: bool) -> Result<File, AuthorityError> {
    let before = if create {
        None
    } else {
        let metadata = fs::symlink_metadata(path).map_err(io_error)?;
        private_file(&metadata)?;
        Some(metadata)
    };
    let file = OpenOptions::new()
        .read(true)
        .write(create)
        .create_new(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(io_error)?;
    check_file(path, &file)?;
    if let Some(before) = before {
        same_inode(&before, &file.metadata().map_err(io_error)?)?;
    }
    if create {
        file.sync_all().map_err(io_error)?;
    }
    Ok(file)
}

fn check_file(path: &Path, file: &File) -> Result<(), AuthorityError> {
    let held = file.metadata().map_err(io_error)?;
    let current = fs::symlink_metadata(path).map_err(io_error)?;
    private_file(&held)?;
    private_file(&current)?;
    same_inode(&held, &current)
}

fn private_directory(metadata: &Metadata) -> Result<(), AuthorityError> {
    if !metadata.is_dir()
        || metadata.mode() & 0o7777 != 0o700
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(AuthorityError::UnsafeStore);
    }
    Ok(())
}

fn private_file(metadata: &Metadata) -> Result<(), AuthorityError> {
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.mode() & 0o7777 != 0o600
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(AuthorityError::UnsafeStore);
    }
    Ok(())
}

fn same_inode(left: &Metadata, right: &Metadata) -> Result<(), AuthorityError> {
    if left.dev() != right.dev() || left.ino() != right.ino() {
        return Err(AuthorityError::UnsafeStore);
    }
    Ok(())
}

fn io_error(error: io::Error) -> AuthorityError {
    if error.kind() == io::ErrorKind::NotFound {
        return AuthorityError::MissingStore;
    }
    if matches!(
        error.raw_os_error(),
        Some(libc::ELOOP) | Some(libc::ENOTDIR)
    ) {
        return AuthorityError::UnsafeStore;
    }
    AuthorityError::Storage
}
