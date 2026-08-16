use crate::policy::journal::{
    content_checksum, migrate_legacy_journal, validate_journal_invariants, verify_content_checksum,
    JournalFileIdentity, JournalPayload, JournalRecord, LEGACY_JOURNAL_SCHEMA_VERSION,
};
use crate::policy::platform::{expected_policy_file_owner, fail_next, sync_directory_fd_strict};
use crate::policy::{journal_path, journal_stage_path, PolicyError, JOURNAL_FILE_MODE};
use anyhow::{bail, Context, Result};
use std::path::Path;

#[cfg(any(test, feature = "sandbox"))]
fn journal_crash_point(point: &str) -> Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static HIT_COUNT: AtomicUsize = AtomicUsize::new(0);
    let requested = std::env::var("QOL_RESIDENT_CRASH_POINT").unwrap_or_default();
    let (name, occurrence) = match requested.split_once(':') {
        Some((name, occurrence)) => (name.to_string(), occurrence.parse::<usize>().unwrap_or(1)),
        None => (requested.clone(), 1),
    };
    if name != point || occurrence == 0 {
        HIT_COUNT.store(0, Ordering::SeqCst);
        return Ok(());
    }
    let hit = HIT_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
    if hit == occurrence {
        unsafe { libc::abort() };
    }
    Ok(())
}

#[cfg(not(any(test, feature = "sandbox")))]
fn journal_crash_point(_point: &str) -> Result<()> {
    Ok(())
}

pub(crate) fn read<T: JournalPayload>(policy: &str) -> Result<Option<JournalRecord<T>>> {
    let canonical = journal_path(policy)?;
    let stage = journal_stage_path(policy)?;
    let parent = canonical
        .parent()
        .context("journal path has no parent directory")?;
    let parent_fd = open_dir_pinned(parent)?;
    let canonical_name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let stage_name = stage
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    if entry_identity(&parent_fd, &stage_name)?.is_some() {
        return Err(PolicyError::JournalInvalid {
            policy: policy.to_string(),
            reason: format!(
                "the journal recovery stage {} exists; the journal write was interrupted or the stage is invalid",
                stage.display()
            ),
        }
        .into());
    }
    match validated_journal_at(&parent_fd, &canonical_name, policy)? {
        Some((journal, _)) => Ok(Some(journal)),
        None => {
            if entry_identity(&parent_fd, &canonical_name)?.is_some() {
                Err(PolicyError::JournalInvalid {
                    policy: policy.to_string(),
                    reason: format!("journal path {} is not a regular file", canonical.display()),
                }
                .into())
            } else {
                Ok(None)
            }
        }
    }
}

pub(crate) fn write_durable<T: JournalPayload>(journal: &JournalRecord<T>) -> Result<()> {
    let policy = journal.policy.clone();
    let canonical = journal_path(&policy)?;
    let stage = journal_stage_path(&policy)?;
    let parent = canonical
        .parent()
        .context("journal path has no parent directory")?;
    let parent_fd = open_dir_pinned(parent)?;
    let canonical_name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let stage_name = stage
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    recover_stage_with_fd::<T>(&parent_fd, &policy)?;
    fail_next("journal-write")?;
    let stage_identity = write_stage_linked(&parent_fd, &stage_name, journal)?;
    if let Err(error) = commit_stage::<T>(
        &parent_fd,
        &canonical_name,
        &stage_name,
        &policy,
        CanonicalGuard::Revalidate,
    ) {
        return Err(cleanup_failed_stage(
            &parent_fd,
            &stage_name,
            stage_identity,
            error,
        ));
    }
    Ok(())
}

pub(crate) fn remove_durable<T: JournalPayload>(policy: &str) -> Result<()> {
    let canonical = journal_path(policy)?;
    let parent = canonical
        .parent()
        .context("journal path has no parent directory")?;
    let parent_fd = open_dir_pinned(parent)?;
    let canonical_name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    recover_stage_with_fd::<T>(&parent_fd, policy)?;
    let Some((_, file_identity)) = validated_journal_at::<T>(&parent_fd, &canonical_name, policy)?
    else {
        if entry_identity(&parent_fd, &canonical_name)?.is_some() {
            return Err(PolicyError::JournalInvalid {
                policy: policy.to_string(),
                reason: format!(
                    "journal path {} exists but is not a validated regular qol journal; it was preserved",
                    canonical.display()
                ),
            }
            .into());
        }
        return Ok(());
    };
    let Some(identity) = file_identity else {
        return Err(PolicyError::JournalInvalid {
            policy: policy.to_string(),
            reason: "the canonical journal carries no file identity; refusing to remove it"
                .to_string(),
        }
        .into());
    };
    #[cfg(any(test, feature = "sandbox"))]
    if std::env::var_os("QOL_JOURNAL_REMOVE_SWAP").is_some() {
        let canonical = journal_path(policy)?;
        let swap = canonical.with_extension("swap");
        std::fs::write(&swap, b"foreign inode bytes")?;
        std::fs::rename(&swap, &canonical)?;
    }
    fail_next("journal-unlink")?;
    match entry_identity(&parent_fd, &canonical_name)? {
        Some(live) if live == (identity.dev, identity.ino) => {}
        Some(_) => {
            bail!("the journal entry changed identity since validation; it was preserved")
        }
        None => return Ok(()),
    }
    unlinkat_ignore_missing(&parent_fd, &canonical_name)?;
    sync_directory_fd_strict(&parent_fd)
}

pub(crate) fn recover_stage<T: JournalPayload>(policy: &str) -> Result<()> {
    let canonical = journal_path(policy)?;
    let parent = canonical
        .parent()
        .context("journal path has no parent directory")?;
    let parent_fd = open_dir_pinned(parent)?;
    recover_stage_with_fd::<T>(&parent_fd, policy)
}

fn open_dir_pinned(path: &Path) -> Result<std::fs::File> {
    use std::ffi::CString;
    use std::os::unix::io::FromRawFd;
    let display = path.display().to_string();
    let path = CString::new(path.as_os_str().as_encoded_bytes())
        .with_context(|| format!("directory path contains a nul byte: {display}"))?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!("failed to open directory {display} without following symlinks")
        });
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to fstat directory {display}"))?;
    if !metadata.is_dir() {
        return Err(PolicyError::JournalInvalid {
            policy: "journal-directory".to_string(),
            reason: format!("journal directory {display} is not a real directory"),
        }
        .into());
    }
    Ok(file)
}

fn fchmod_file(file: &std::fs::File, mode: u32) -> Result<()> {
    use std::os::unix::io::AsRawFd;
    let result = unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) };
    if result == 0 {
        return Ok(());
    }
    Err(std::io::Error::last_os_error())
        .with_context(|| format!("failed to apply journal mode {mode:o} to an owned artifact"))
}

fn entry_identity(dir_fd: &std::fs::File, name: &str) -> Result<Option<(u64, u64)>> {
    use std::ffi::CString;
    use std::os::unix::io::AsRawFd;
    let name = CString::new(name).context("journal entry name has a nul byte")?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            dir_fd.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        let stat = unsafe { stat.assume_init() };
        return Ok(Some((stat.st_dev, stat.st_ino)));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        return Ok(None);
    }
    Err(error).with_context(|| format!("failed to fstat journal entry {name:?}"))
}

fn unlinkat_ignore_missing(fd: &std::fs::File, name: &str) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::io::AsRawFd;
    let name = CString::new(name).context("journal temp name contains a nul byte")?;
    let result = unsafe { libc::unlinkat(fd.as_raw_fd(), name.as_ptr(), 0) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        return Ok(());
    }
    Err(error).with_context(|| format!("failed to remove journal temp {name:?}"))
}

fn validated_journal_at<T: JournalPayload>(
    dir_fd: &std::fs::File,
    name: &str,
    policy: &str,
) -> Result<Option<(JournalRecord<T>, Option<JournalFileIdentity>)>> {
    if name != format!("qol-resident-policy-{policy}.json") {
        return Ok(None);
    }
    let Some((content, stats)) = read_regular_at_identity(dir_fd, name)? else {
        return Ok(None);
    };
    let mut journal: JournalRecord<T> = serde_json::from_slice(&content)
        .with_context(|| format!("failed to parse journal entry {name:?}"))?;
    if journal.policy != policy {
        return Err(PolicyError::JournalInvalid {
            policy: policy.to_string(),
            reason: format!(
                "the journal file names policy `{}` but embeds policy `{}`",
                policy, journal.policy
            ),
        }
        .into());
    }
    let context = format!("journal entry {name:?}");
    let legacy = journal.schema_version == LEGACY_JOURNAL_SCHEMA_VERSION;
    let migrated = migrate_legacy_journal(&mut journal, &context)?;
    verify_content_checksum(&journal, policy, &context, legacy)?;
    validate_journal_invariants(&journal)?;
    validate_on_disk_journal(&stats, &journal, policy, &context)?;
    let file_identity = if migrated {
        let (dev, ino) =
            rewrite_migrated_journal::<T>(dir_fd, name, policy, &journal, (stats.dev, stats.ino))?;
        journal.journal_file_identity = Some(JournalFileIdentity { dev, ino });
        journal.content_sha256 = content_checksum(&journal)?;
        journal.journal_file_identity
    } else {
        journal.journal_file_identity
    };
    Ok(Some((journal, file_identity)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct JournalFileStats {
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

fn read_regular_at_identity(
    dir: &std::fs::File,
    name: &str,
) -> Result<Option<(Vec<u8>, JournalFileStats)>> {
    use std::ffi::CString;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::io::{AsRawFd, FromRawFd};
    const MAX_JOURNAL_BYTES: usize = 64 * 1024;
    let name = CString::new(name).context("journal name contains a nul byte")?;
    let pinned = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if pinned < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ELOOP)
        {
            return Ok(None);
        }
        return Err(error).with_context(|| format!("failed to pin journal entry {name:?}"));
    }
    let pinned_file = unsafe { std::fs::File::from_raw_fd(pinned) };
    let pinned_metadata = pinned_file
        .metadata()
        .with_context(|| format!("failed to fstat the pinned journal entry {name:?}"))?;
    if !pinned_metadata.is_file() {
        return Ok(None);
    }
    let proc_link = format!("/proc/self/fd/{}", pinned_file.as_raw_fd());
    let proc_c =
        CString::new(proc_link.as_str()).context("proc fd link path contains a nul byte")?;
    let read_fd = unsafe { libc::open(proc_c.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if read_fd < 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!("failed to open the pinned journal entry {name:?} for reading")
        });
    }
    let file = unsafe { std::fs::File::from_raw_fd(read_fd) };
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to fstat the opened journal entry {name:?}"))?;
    if metadata.dev() != pinned_metadata.dev() || metadata.ino() != pinned_metadata.ino() {
        return Err(anyhow::anyhow!(
            "the opened journal entry {name:?} is not the pinned identity; it was preserved"
        ));
    }
    let mut content = Vec::new();
    {
        use std::io::Read;
        file.take(MAX_JOURNAL_BYTES as u64 + 1)
            .read_to_end(&mut content)
            .with_context(|| format!("failed to read journal entry {name:?}"))?;
    }
    if content.len() > MAX_JOURNAL_BYTES {
        return Err(PolicyError::JournalInvalid {
            policy: "journal-entry".to_string(),
            reason: format!("journal entry {name:?} exceeds the {MAX_JOURNAL_BYTES}-byte bound"),
        }
        .into());
    }
    Ok(Some((
        content,
        JournalFileStats {
            dev: metadata.dev(),
            ino: metadata.ino(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
        },
    )))
}

fn validate_on_disk_journal<T>(
    stats: &JournalFileStats,
    journal: &JournalRecord<T>,
    policy: &str,
    context: &str,
) -> Result<()> {
    let file_identity =
        journal
            .journal_file_identity
            .as_ref()
            .ok_or_else(|| PolicyError::JournalInvalid {
                policy: policy.to_string(),
                reason: format!("{context} carries no embedded file identity; it was preserved"),
            })?;
    if file_identity.dev != stats.dev || file_identity.ino != stats.ino {
        return Err(PolicyError::JournalInvalid {
            policy: policy.to_string(),
            reason: format!(
                "{context} is not the exact file whose identity the journal embeds; it was preserved"
            ),
        }
        .into());
    }
    if stats.mode & 0o7777 != JOURNAL_FILE_MODE {
        return Err(PolicyError::JournalInvalid {
            policy: policy.to_string(),
            reason: format!(
                "{context} carries mode {:o} instead of the exact {JOURNAL_FILE_MODE:o}; it was preserved",
                stats.mode & 0o7777
            ),
        }
        .into());
    }
    let (expected_uid, expected_gid) = expected_policy_file_owner();
    if stats.uid != expected_uid || stats.gid != expected_gid {
        return Err(PolicyError::JournalInvalid {
            policy: policy.to_string(),
            reason: format!(
                "{context} carries uid {} gid {} instead of the exact {expected_uid}:{expected_gid}; it was preserved",
                stats.uid, stats.gid
            ),
        }
        .into());
    }
    Ok(())
}

enum JournalStage {
    Absent,
    Recoverable((u64, u64)),
    Unrecoverable,
}

fn journal_stage_state<T: JournalPayload>(
    parent_fd: &std::fs::File,
    stage_name: &str,
    policy: &str,
) -> Result<JournalStage> {
    if entry_identity(parent_fd, stage_name)?.is_none() {
        return Ok(JournalStage::Absent);
    }
    let Some((content, stats)) = read_regular_at_identity(parent_fd, stage_name)? else {
        return Ok(JournalStage::Unrecoverable);
    };
    let journal: JournalRecord<T> = serde_json::from_slice(&content)
        .with_context(|| format!("failed to parse the journal recovery stage {stage_name:?}"))?;
    if journal.policy != policy {
        return Ok(JournalStage::Unrecoverable);
    }
    if validate_journal_invariants(&journal).is_err()
        || verify_content_checksum(
            &journal,
            policy,
            &format!("journal recovery stage {stage_name:?}"),
            false,
        )
        .is_err()
        || validate_on_disk_journal(
            &stats,
            &journal,
            policy,
            &format!("journal recovery stage {stage_name:?}"),
        )
        .is_err()
    {
        return Ok(JournalStage::Unrecoverable);
    }
    Ok(JournalStage::Recoverable((stats.dev, stats.ino)))
}

fn recover_stage_with_fd<T: JournalPayload>(parent_fd: &std::fs::File, policy: &str) -> Result<()> {
    let stage = journal_stage_path(policy)?;
    let stage_name = stage
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    match journal_stage_state::<T>(parent_fd, &stage_name, policy)? {
        JournalStage::Absent => {}
        JournalStage::Recoverable((dev, ino)) => {
            fail_next("stage-recover-remove")?;
            match entry_identity(parent_fd, &stage_name)? {
                Some(live) if live == (dev, ino) => {}
                Some(_) => bail!(
                    "the recoverable journal stage changed identity before removal; it was preserved"
                ),
                None => return Ok(()),
            }
            unlinkat_ignore_missing(parent_fd, &stage_name)?;
            sync_directory_fd_strict(parent_fd)?;
        }
        JournalStage::Unrecoverable => {
            return Err(PolicyError::JournalInvalid {
                policy: policy.to_string(),
                reason: format!(
                    "the journal recovery stage {} exists but is not a recoverable qol journal for this policy; it was preserved",
                    stage.display()
                ),
            }
            .into());
        }
    }
    Ok(())
}

fn write_stage_linked<T: JournalPayload>(
    parent_fd: &std::fs::File,
    stage_name: &str,
    journal: &JournalRecord<T>,
) -> Result<(u64, u64)> {
    use std::ffi::CString;
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::io::{AsRawFd, FromRawFd};
    let fd = unsafe {
        libc::openat(
            parent_fd.as_raw_fd(),
            c".".as_ptr(),
            libc::O_TMPFILE | libc::O_WRONLY | libc::O_CLOEXEC,
            JOURNAL_FILE_MODE as libc::mode_t,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| "failed to create the unnamed journal temp file");
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    fchmod_file(&file, JOURNAL_FILE_MODE)?;
    let (owner_uid, owner_gid) = expected_policy_file_owner();
    let owner = unsafe { libc::fchown(file.as_raw_fd(), owner_uid, owner_gid) };
    if owner != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| "failed to set the exact journal ownership on the temp");
    }
    let metadata = file
        .metadata()
        .with_context(|| "failed to fstat the journal temp")?;
    let identity = JournalFileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    };
    let mut journal = journal.clone();
    journal.journal_file_identity = Some(identity);
    journal.content_sha256 = content_checksum(&journal)?;
    let content = serde_json::to_vec(&journal).context("failed to serialize the journal")?;
    file.write_all(&content)
        .with_context(|| "failed to write the journal temp")?;
    fail_next("journal-file-sync")?;
    file.sync_all()
        .with_context(|| "failed to fsync the journal temp")?;
    let stage = CString::new(stage_name).context("journal stage name has a nul byte")?;
    let linked = unsafe {
        libc::linkat(
            file.as_raw_fd(),
            c"".as_ptr(),
            parent_fd.as_raw_fd(),
            stage.as_ptr(),
            libc::AT_EMPTY_PATH,
        )
    };
    if linked != 0 {
        let error = std::io::Error::last_os_error();
        let unprivileged = matches!(
            error.raw_os_error(),
            Some(libc::ENOENT) | Some(libc::EPERM) | Some(libc::EACCES)
        );
        if !unprivileged {
            return Err(error).with_context(|| {
                format!("failed to link the journal stage {stage_name:?} without replacing")
            });
        }
        let proc_link = format!("/proc/self/fd/{}", file.as_raw_fd());
        let proc_c =
            CString::new(proc_link.as_str()).context("proc fd link path contains a nul byte")?;
        let linked = unsafe {
            libc::linkat(
                parent_fd.as_raw_fd(),
                proc_c.as_ptr(),
                parent_fd.as_raw_fd(),
                stage.as_ptr(),
                libc::AT_SYMLINK_FOLLOW,
            )
        };
        if linked != 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| {
                format!(
                    "failed to link the owned descriptor into the journal stage {stage_name:?} without replacing"
                )
            });
        }
    }
    let stage_identity = (metadata.dev(), metadata.ino());
    match entry_identity(parent_fd, stage_name)? {
        Some(live) if live == stage_identity => {}
        Some(_) => bail!(
            "the linked journal stage {stage_name:?} is not the exact descriptor identity; it was preserved"
        ),
        None => bail!("the linked journal stage {stage_name:?} vanished immediately"),
    }
    journal_crash_point("after-journal-stage-link")?;
    if let Err(error) = sync_directory_fd_strict(parent_fd) {
        let mut cleanup = Vec::new();
        match entry_identity(parent_fd, stage_name) {
            Ok(Some(live)) if live == stage_identity => {
                if let Err(unlink_error) = unlinkat_ignore_missing(parent_fd, stage_name) {
                    cleanup.push(unlink_error);
                }
                if let Err(sync_error) = sync_directory_fd_strict(parent_fd) {
                    cleanup.push(sync_error);
                }
            }
            Ok(_) => {}
            Err(identity_error) => cleanup.push(identity_error),
        }
        if cleanup.is_empty() {
            return Err(error);
        }
        let details = cleanup
            .iter()
            .map(|error| format!("{error:#}"))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(anyhow::anyhow!(
            "{error:#}; additionally, stage cleanup failed: {details}"
        ));
    }
    Ok(stage_identity)
}

fn renameat2_noreplace(dir_fd: &std::fs::File, from: &str, to: &str) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::io::AsRawFd;
    let from = CString::new(from).context("journal stage name contains a nul byte")?;
    let to = CString::new(to).context("journal name contains a nul byte")?;
    let result = unsafe {
        libc::renameat2(
            dir_fd.as_raw_fd(),
            from.as_ptr(),
            dir_fd.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    Err(std::io::Error::last_os_error()).with_context(|| {
        format!("failed to commit the journal rename {from:?} -> {to:?} without replacing")
    })
}

fn renameat_replace(dir_fd: &std::fs::File, from: &str, to: &str) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::io::AsRawFd;
    let from = CString::new(from).context("journal stage name contains a nul byte")?;
    let to = CString::new(to).context("journal name contains a nul byte")?;
    let result = unsafe {
        libc::renameat(
            dir_fd.as_raw_fd(),
            from.as_ptr(),
            dir_fd.as_raw_fd(),
            to.as_ptr(),
        )
    };
    if result == 0 {
        return Ok(());
    }
    Err(std::io::Error::last_os_error())
        .with_context(|| format!("failed to commit journal rename {from:?} -> {to:?}"))
}

fn validated_canonical_identity<T: JournalPayload>(
    parent_fd: &std::fs::File,
    canonical_name: &str,
    policy: &str,
) -> Result<(u64, u64)> {
    let Some((content, stats)) = read_regular_at_identity(parent_fd, canonical_name)? else {
        bail!("the canonical journal is not a regular file; refusing to replace it");
    };
    let journal: JournalRecord<T> = serde_json::from_slice(&content)
        .with_context(|| format!("failed to parse the canonical journal {canonical_name:?}"))?;
    if journal.policy != policy {
        bail!("the canonical journal names a different policy; refusing to replace it");
    }
    validate_journal_invariants(&journal)?;
    verify_content_checksum(
        &journal,
        policy,
        &format!("the canonical journal {canonical_name:?}"),
        false,
    )?;
    validate_on_disk_journal(
        &stats,
        &journal,
        policy,
        &format!("the canonical journal {canonical_name:?}"),
    )?;
    Ok((stats.dev, stats.ino))
}

fn cleanup_failed_stage(
    parent_fd: &std::fs::File,
    stage_name: &str,
    stage_identity: (u64, u64),
    primary: anyhow::Error,
) -> anyhow::Error {
    let mut cleanup = Vec::new();
    match entry_identity(parent_fd, stage_name) {
        Ok(Some(live)) if live == stage_identity => {
            if let Err(unlink_error) = unlinkat_ignore_missing(parent_fd, stage_name) {
                cleanup.push(unlink_error);
            }
            if let Err(sync_error) = sync_directory_fd_strict(parent_fd) {
                cleanup.push(sync_error);
            }
        }
        Ok(_) => {}
        Err(identity_error) => cleanup.push(identity_error),
    }
    if cleanup.is_empty() {
        return primary;
    }
    let details = cleanup
        .iter()
        .map(|error| format!("{error:#}"))
        .collect::<Vec<_>>()
        .join("; ");
    anyhow::anyhow!("{primary:#}; additionally, stage cleanup failed: {details}")
}

enum CanonicalGuard {
    Revalidate,
    KnownIdentity((u64, u64)),
}

fn commit_stage<T: JournalPayload>(
    parent_fd: &std::fs::File,
    canonical_name: &str,
    stage_name: &str,
    policy: &str,
    guard: CanonicalGuard,
) -> Result<()> {
    match entry_identity(parent_fd, canonical_name)? {
        None => {
            fail_next("journal-first-commit")?;
            renameat2_noreplace(parent_fd, stage_name, canonical_name)?;
        }
        Some(_) => {
            let (dev, ino) = match guard {
                CanonicalGuard::Revalidate => {
                    validated_canonical_identity::<T>(parent_fd, canonical_name, policy)?
                }
                CanonicalGuard::KnownIdentity(identity) => identity,
            };
            #[cfg(any(test, feature = "sandbox"))]
            if let Some(replacement) = std::env::var_os("QOL_JOURNAL_REVALIDATE_SWAP") {
                let canonical = journal_path(policy)?;
                let swap = canonical.with_extension("swap");
                std::fs::write(&swap, replacement.as_encoded_bytes())?;
                std::fs::rename(&swap, &canonical)?;
            }
            fail_next("journal-update-revalidate")?;
            let live = entry_identity(parent_fd, canonical_name)?.ok_or_else(|| {
                anyhow::anyhow!("the canonical journal vanished before replacement")
            })?;
            if live != (dev, ino) {
                bail!(
                    "the canonical journal changed identity before replacement; refusing to clobber it"
                );
            }
            fail_next("journal-update-rename")?;
            renameat_replace(parent_fd, stage_name, canonical_name)?;
        }
    }
    fail_next("journal-dir-sync")?;
    sync_directory_fd_strict(parent_fd)
}

fn rewrite_migrated_journal<T: JournalPayload>(
    parent_fd: &std::fs::File,
    canonical_name: &str,
    policy: &str,
    journal: &JournalRecord<T>,
    original_identity: (u64, u64),
) -> Result<(u64, u64)> {
    let stage = journal_stage_path(policy)?;
    let stage_name = stage
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    fail_next("journal-migration-write")?;
    let stage_identity = write_stage_linked(parent_fd, &stage_name, journal)?;
    if let Err(error) = commit_stage::<T>(
        parent_fd,
        canonical_name,
        &stage_name,
        policy,
        CanonicalGuard::KnownIdentity(original_identity),
    ) {
        return Err(cleanup_failed_stage(
            parent_fd,
            &stage_name,
            stage_identity,
            error,
        ));
    }
    Ok(stage_identity)
}
