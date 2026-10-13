use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions, TryLockError},
    io,
    os::windows::{
        ffi::OsStringExt,
        fs::{FileExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
};

use qol_platform::native::security;
use windows_sys::Win32::Foundation::{ERROR_CANT_RESOLVE_FILENAME, ERROR_DIRECTORY};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, READ_CONTROL,
    VOLUME_NAME_DOS, WRITE_DAC,
};
use zeroize::Zeroizing;

use crate::service::authority::{state::MAX_SNAPSHOT_BYTES, AuthorityError};

const NO_FOLLOW: u32 = FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT;

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
        fs::create_dir(root).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                return AuthorityError::AlreadyExists;
            }
            io_error(error)
        })?;
        let directory = open(root, READ_CONTROL | WRITE_DAC)?;
        security::restrict_to_owner(&directory).map_err(io_error)?;
        Self::acquire(root, true)
    }

    pub fn open(root: &Path) -> Result<Self, AuthorityError> {
        Self::acquire(root, false)
    }

    fn acquire(root: &Path, create: bool) -> Result<Self, AuthorityError> {
        let root: PathBuf = root.components().collect();
        let directory = open(&root, READ_CONTROL | FILE_READ_ATTRIBUTES)?;
        private_directory(&directory)?;
        let canonical_root = final_path(&directory)?;
        let lock_path = canonical_root.join("writer.lock");
        let lock = open_regular(&lock_path, create)?;
        lock.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => AuthorityError::WriterBusy,
            TryLockError::Error(_) => AuthorityError::Storage,
        })?;
        let snapshot = if create {
            None
        } else {
            Some(open_regular(&canonical_root.join("state.json"), false)?)
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
        read_exact_at(file, &mut bytes[..], 0)?;
        let mut extra = Zeroizing::new([0_u8; 1]);
        if file.seek_read(extra.as_mut(), length).map_err(io_error)? != 0 {
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
        let path = final_path(&self.directory)?.join("state.json");
        qol_fs::atomic_write_private(&path, bytes).map_err(io_error)?;
        #[cfg(test)]
        if matches!(fault, Some(super::CommitFault::AfterReplace)) {
            return Err(AuthorityError::Storage);
        }
        self.snapshot = Some(open_regular(&path, false)?);
        self.validate()
    }

    fn validate(&self) -> Result<(), AuthorityError> {
        private_directory(&self.directory)?;
        same_file(
            &self.directory,
            &open(&self.canonical_root, FILE_READ_ATTRIBUTES)?,
        )?;
        let anchored = final_path(&self.directory)?;
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

fn open(path: &Path, access: u32) -> Result<File, AuthorityError> {
    OpenOptions::new()
        .access_mode(access)
        .custom_flags(NO_FOLLOW)
        .open(path)
        .map_err(io_error)
}

fn open_regular(path: &Path, create: bool) -> Result<File, AuthorityError> {
    let file = OpenOptions::new()
        .read(true)
        .write(create)
        .create_new(create)
        .custom_flags(NO_FOLLOW)
        .open(path)
        .map_err(io_error)?;
    check_file(path, &file)?;
    Ok(file)
}

fn check_file(path: &Path, file: &File) -> Result<(), AuthorityError> {
    let held = information(file)?;
    if held.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
        || held.nNumberOfLinks != 1
        || !owner_only(file)?
    {
        return Err(AuthorityError::UnsafeStore);
    }
    same_file(file, &open(path, FILE_READ_ATTRIBUTES)?)
}

fn private_directory(directory: &File) -> Result<(), AuthorityError> {
    let attributes = information(directory)?.dwFileAttributes;
    if attributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || !owner_only(directory)?
    {
        return Err(AuthorityError::UnsafeStore);
    }
    Ok(())
}

fn owner_only(file: &File) -> Result<bool, AuthorityError> {
    security::is_owner_only(file).map_err(io_error)
}

fn same_file(left: &File, right: &File) -> Result<(), AuthorityError> {
    let left = information(left)?;
    let right = information(right)?;
    if left.dwVolumeSerialNumber != right.dwVolumeSerialNumber
        || left.nFileIndexHigh != right.nFileIndexHigh
        || left.nFileIndexLow != right.nFileIndexLow
    {
        return Err(AuthorityError::UnsafeStore);
    }
    Ok(())
}

fn information(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION, AuthorityError> {
    let mut information = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(io_error(io::Error::last_os_error()));
    }
    Ok(information)
}

fn final_path(file: &File) -> Result<PathBuf, AuthorityError> {
    let mut buffer = vec![0u16; 512];
    loop {
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        } as usize;
        if length == 0 {
            return Err(io_error(io::Error::last_os_error()));
        }
        if length < buffer.len() {
            return Ok(PathBuf::from(OsString::from_wide(&buffer[..length])));
        }
        buffer.resize(length, 0);
    }
}

fn read_exact_at(
    file: &File,
    mut buffer: &mut [u8],
    mut offset: u64,
) -> Result<(), AuthorityError> {
    while !buffer.is_empty() {
        let read = file.seek_read(buffer, offset).map_err(io_error)?;
        if read == 0 {
            return Err(AuthorityError::Storage);
        }
        buffer = &mut buffer[read..];
        offset += read as u64;
    }
    Ok(())
}

fn io_error(error: io::Error) -> AuthorityError {
    if error.kind() == io::ErrorKind::NotFound {
        return AuthorityError::MissingStore;
    }
    if matches!(
        error.raw_os_error(),
        Some(code) if code == ERROR_DIRECTORY as i32 || code == ERROR_CANT_RESOLVE_FILENAME as i32
    ) {
        return AuthorityError::UnsafeStore;
    }
    AuthorityError::Storage
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    fn created() -> (tempfile::TempDir, PathBuf, Store) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("authority");
        let mut store = Store::create(&root).unwrap();
        store.write(b"first").unwrap();
        (temporary, root, store)
    }

    fn junction(link: &Path, target: &Path) {
        let made = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(made.status.success(), "{}", link.display());
    }

    fn grant_everyone(path: &Path) {
        let granted = Command::new("icacls")
            .arg(path)
            .args(["/grant", "*S-1-1-0:R"])
            .output()
            .unwrap();
        assert!(granted.status.success(), "{}", path.display());
    }

    #[test]
    fn store_files_are_owner_only_single_links_and_keep_their_lock() {
        let (_temporary, root, mut store) = created();
        assert!(owner_only(&open(&root, READ_CONTROL).unwrap()).unwrap());
        for name in ["writer.lock", "state.json"] {
            let file = open(&root.join(name), READ_CONTROL | FILE_READ_ATTRIBUTES).unwrap();
            assert!(owner_only(&file).unwrap(), "{name}");
            assert_eq!(information(&file).unwrap().nNumberOfLinks, 1, "{name}");
        }
        let lock = open(&root.join("writer.lock"), FILE_READ_ATTRIBUTES).unwrap();
        store.write(b"second").unwrap();
        same_file(
            &lock,
            &open(&root.join("writer.lock"), FILE_READ_ATTRIBUTES).unwrap(),
        )
        .unwrap();
        drop(store);
        assert_eq!(&Store::open(&root).unwrap().read().unwrap()[..], b"second");
    }

    #[test]
    fn a_second_writer_is_busy() {
        let (_temporary, root, _store) = created();
        assert!(matches!(
            Store::open(&root),
            Err(AuthorityError::WriterBusy)
        ));
    }

    #[test]
    fn an_extra_grant_is_rejected_without_repair() {
        for name in ["", "state.json", "writer.lock"] {
            let (_temporary, root, store) = created();
            drop(store);
            let path = if name.is_empty() {
                root.clone()
            } else {
                root.join(name)
            };
            grant_everyone(&path);
            assert!(
                matches!(Store::open(&root), Err(AuthorityError::UnsafeStore)),
                "{name}"
            );
            assert!(
                !owner_only(&open(&path, READ_CONTROL).unwrap()).unwrap(),
                "{name}"
            );
        }
    }

    #[test]
    fn links_and_directories_in_place_of_store_files_are_rejected() {
        for name in ["state.json", "writer.lock"] {
            for kind in ["junction", "hardlink", "directory"] {
                let (temporary, root, store) = created();
                drop(store);
                let path = root.join(name);
                let saved = temporary.path().join("saved");
                fs::rename(&path, &saved).unwrap();
                match kind {
                    "junction" => junction(&path, temporary.path()),
                    "hardlink" => fs::hard_link(&saved, &path).unwrap(),
                    "directory" => fs::create_dir(&path).unwrap(),
                    other => panic!("unknown fixture {other}"),
                }
                assert!(
                    matches!(Store::open(&root), Err(AuthorityError::UnsafeStore)),
                    "{name} {kind}"
                );
                assert!(saved.is_file());
            }
        }
        let (temporary, root, store) = created();
        drop(store);
        let alias = temporary.path().join("root-link");
        junction(&alias, &root);
        assert!(matches!(
            Store::open(&alias),
            Err(AuthorityError::UnsafeStore)
        ));
    }

    #[test]
    fn a_live_writer_pins_its_root() {
        let (temporary, root, mut store) = created();
        assert!(fs::rename(&root, temporary.path().join("saved")).is_err());
        store.write(b"second").unwrap();
        assert_eq!(fs::read(root.join("state.json")).unwrap(), b"second");
    }
}
