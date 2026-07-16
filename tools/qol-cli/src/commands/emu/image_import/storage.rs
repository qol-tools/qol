use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::plan::{validate_qcow2, SourceStamp};
use super::{ImageImportPlan, ImportCancellation};

const PROCESS_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const CONVERT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const PROMOTION_LOCK_TIMEOUT: Duration = Duration::from_secs(300);
const PROMOTION_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StagedImage {
    pub(super) path: PathBuf,
    pub(super) image_path: PathBuf,
    pub(super) sha256: String,
    pub(super) size_bytes: u64,
    pub(super) virtual_size: u64,
}

#[derive(Debug)]
pub(super) struct StageSourceFailure {
    pub(super) error: anyhow::Error,
    pub(super) process_tree_exit_verified: bool,
}

struct ConversionJournal<'a> {
    run_id: &'a str,
    program: &'a Path,
    source: &'a Path,
    destination: &'a Path,
    state: &'a str,
    pid: Option<u32>,
    process_identity: Option<&'a str>,
    tree_exit_verified: bool,
    error: Option<&'a str>,
}

impl StageSourceFailure {
    fn complete(error: anyhow::Error) -> Self {
        Self {
            error,
            process_tree_exit_verified: true,
        }
    }

    fn incomplete(error: anyhow::Error) -> Self {
        Self {
            error,
            process_tree_exit_verified: false,
        }
    }
}

impl From<anyhow::Error> for StageSourceFailure {
    fn from(error: anyhow::Error) -> Self {
        Self::complete(error)
    }
}

pub(super) fn stage_source(
    plan: &ImageImportPlan,
    stage_path: &Path,
    cancellation: &ImportCancellation,
    verbose: bool,
) -> std::result::Result<StagedImage, StageSourceFailure> {
    let before = SourceStamp::read(&plan.source)?;
    if before != plan.source_stamp {
        return Err(StageSourceFailure::complete(anyhow!(
            "source image changed after import planning; create a new import plan"
        )));
    }
    let qemu_img = super::super::resolve_backend(plan.backend)
        .map_err(anyhow::Error::msg)?
        .qemu_img
        .context("verified image import requires qemu-img")?;
    convert_sparse(
        &plan.run_id,
        &qemu_img,
        &plan.source,
        stage_path,
        cancellation,
        verbose,
    )?;
    let after = SourceStamp::read(&plan.source)?;
    if before != after {
        return Err(StageSourceFailure::complete(anyhow!(
            "source image changed while it was staged; retry with a stable source file"
        )));
    }
    let staged_info = super::super::inspect_image(&qemu_img, stage_path, verbose)?;
    validate_qcow2(&staged_info, stage_path)?;
    if staged_info.virtual_size != plan.source_virtual_size {
        return Err(StageSourceFailure::complete(anyhow!(
            "staged image virtual size changed during conversion"
        )));
    }
    let size_bytes = fs::metadata(stage_path)
        .with_context(|| format!("failed to inspect {}", stage_path.display()))?
        .len();
    if size_bytes == 0 {
        return Err(StageSourceFailure::complete(anyhow!(
            "staged image is empty"
        )));
    }
    let sha256 = sha256_file(stage_path, || cancellation.is_requested())?;
    let image_path = qol_dev_env::managed_verified_image_path(&plan.image_root, &sha256)?;
    set_readonly(stage_path)?;
    Ok(StagedImage {
        path: stage_path.to_path_buf(),
        image_path,
        sha256,
        size_bytes,
        virtual_size: staged_info.virtual_size,
    })
}

pub(super) fn promote_image(
    plan: &ImageImportPlan,
    staged: &StagedImage,
    cancelled: impl Fn() -> bool,
) -> Result<Value> {
    let parent = staged
        .image_path
        .parent()
        .context("managed image path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let lock_path = parent.join(format!(".{}.lock", staged.sha256));
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open {}", lock_path.display()))?;
    acquire_promotion_lock(&lock, &lock_path, &cancelled)?;
    if cancelled() {
        bail!("image import cancelled before managed image publication");
    }
    let reused = match fs::hard_link(&staged.path, &staged.image_path) {
        Ok(()) => false,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            verify_existing_image(staged, &cancelled)?;
            true
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to publish managed image {}",
                    staged.image_path.display()
                )
            })
        }
    };
    set_readonly(&staged.image_path)?;
    qol_fs::sync_directory(parent)
        .with_context(|| format!("failed to sync {}", parent.display()))?;
    Ok(json!({
        "status": "published",
        "image_path": staged.image_path,
        "reused": reused,
        "image_root": plan.image_root,
    }))
}

pub(super) fn remove_stage(path: &Path) -> Result<()> {
    #[cfg(windows)]
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        if permissions.readonly() {
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions).with_context(|| {
                format!("failed to make staged image {} removable", path.display())
            })?;
        }
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to remove staged image {}", path.display()))
        }
    }
}

pub(super) fn remove_promoted_stage(staged: &StagedImage) -> Result<()> {
    remove_stage(&staged.path)?;
    set_readonly(&staged.image_path)?;
    let permissions = fs::metadata(&staged.image_path)
        .with_context(|| format!("failed to inspect {}", staged.image_path.display()))?
        .permissions();
    if !permissions.readonly() {
        bail!(
            "managed image is writable after stage cleanup: {}",
            staged.image_path.display()
        );
    }
    if let Some(parent) = staged.image_path.parent() {
        qol_fs::sync_directory(parent)
            .with_context(|| format!("failed to sync {}", parent.display()))?;
    }
    Ok(())
}

fn acquire_promotion_lock(
    lock: &File,
    lock_path: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<()> {
    let deadline = Instant::now()
        .checked_add(PROMOTION_LOCK_TIMEOUT)
        .context("image promotion lock timeout is too large")?;
    loop {
        if cancelled() {
            bail!("image import cancelled while waiting to publish the managed image");
        }
        match lock.try_lock() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("failed to lock {}", lock_path.display()))
            }
        }
        let now = Instant::now();
        if now >= deadline {
            bail!(
                "timed out after {PROMOTION_LOCK_TIMEOUT:?} acquiring image promotion lock {}",
                lock_path.display()
            );
        }
        thread::sleep(PROMOTION_LOCK_POLL_INTERVAL.min(deadline.duration_since(now)));
    }
}

fn convert_sparse(
    run_id: &str,
    qemu_img: &Path,
    source: &Path,
    destination: &Path,
    cancellation: &ImportCancellation,
    verbose: bool,
) -> std::result::Result<(), StageSourceFailure> {
    if fs::symlink_metadata(destination).is_ok() {
        return Err(StageSourceFailure::complete(anyhow!(
            "staged image already exists: {}",
            destination.display()
        )));
    }
    write_conversion_journal(ConversionJournal {
        run_id,
        program: qemu_img,
        source,
        destination,
        state: "launching",
        pid: None,
        process_identity: None,
        tree_exit_verified: false,
        error: None,
    })
    .map_err(StageSourceFailure::complete)?;
    let log_path = destination.with_extension("convert.log");
    let log = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&log_path)
        .with_context(|| format!("failed to create {}", log_path.display()))?;
    let stderr = log
        .try_clone()
        .with_context(|| format!("failed to clone {}", log_path.display()))?;
    let args = convert_args(source, destination);
    let mut command = Command::new(qemu_img);
    command
        .args(&args)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    qol_process::isolate_owned_command(&mut command)
        .context("failed to isolate qemu-img process tree")?;
    let process_tree = qol_process::own_current_process_tree()
        .context("failed to create qemu-img process-tree ownership")?;
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let error = anyhow!(error).context(format!("failed to spawn {}", qemu_img.display()));
            let detail = format!("{error:#}");
            write_conversion_journal(ConversionJournal {
                run_id,
                program: qemu_img,
                source,
                destination,
                state: "failed",
                pid: None,
                process_identity: None,
                tree_exit_verified: true,
                error: Some(&detail),
            })
            .map_err(StageSourceFailure::complete)?;
            return Err(StageSourceFailure::complete(error));
        }
    };
    let child_pid = child.id();
    let child_identity = qol_process::process_identity(child_pid).ok();
    if let Err(error) = process_tree.assign(&child) {
        let cleanup = terminate_conversion_process(&mut child, None);
        let failure = anyhow!(
            "failed to own qemu-img process tree: {error}; fallback cleanup: {}",
            cleanup.err().map_or_else(
                || "direct child stopped without tree proof".to_string(),
                |error| { format!("{error:#}") }
            )
        );
        let detail = format!("{failure:#}");
        write_conversion_journal(ConversionJournal {
            run_id,
            program: qemu_img,
            source,
            destination,
            state: "cleanup-incomplete",
            pid: Some(child_pid),
            process_identity: child_identity.as_deref(),
            tree_exit_verified: false,
            error: Some(&detail),
        })
        .map_err(|journal| {
            StageSourceFailure::incomplete(anyhow!(
                "{failure:#}; conversion journal update failed: {journal:#}"
            ))
        })?;
        return Err(StageSourceFailure::incomplete(failure));
    }
    if let Err(journal) = write_conversion_journal(ConversionJournal {
        run_id,
        program: qemu_img,
        source,
        destination,
        state: "running",
        pid: Some(child_pid),
        process_identity: child_identity.as_deref(),
        tree_exit_verified: false,
        error: None,
    }) {
        let cleanup = terminate_conversion_process(&mut child, Some(&process_tree));
        return Err(match cleanup {
            Ok(()) => StageSourceFailure::complete(journal),
            Err(cleanup) => StageSourceFailure::incomplete(anyhow!(
                "conversion journal update failed: {journal:#}; process-tree cleanup failed: {cleanup:#}"
            )),
        });
    }
    let outcome = loop {
        if cancellation.is_requested() {
            break Err(anyhow!(
                "image import cancelled while staging the source image"
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(CONVERT_POLL_INTERVAL),
            Err(error) => break Err(anyhow!(error).context("failed to wait for qemu-img")),
        }
    };
    let cleanup = terminate_conversion_process(&mut child, Some(&process_tree));
    let detail = fs::read_to_string(&log_path).unwrap_or_default();
    let outcome = outcome.and_then(|status| {
        if status.success() {
            return Ok(());
        }
        Err(anyhow!(
            "qemu-img sparse conversion failed with {status}: {}",
            detail.trim()
        ))
    });
    if verbose && outcome.is_err() && !detail.is_empty() {
        eprint!("{detail}");
    }
    let result = match (outcome, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(StageSourceFailure::complete(error)),
        (Ok(()), Err(cleanup)) => Err(StageSourceFailure::incomplete(cleanup)),
        (Err(error), Err(cleanup)) => Err(StageSourceFailure::incomplete(anyhow!(
            "{error:#}; process-tree cleanup failed: {cleanup:#}"
        ))),
    };
    let (state, tree_exit_verified, error) = match &result {
        Ok(()) => ("complete", true, None),
        Err(failure) if failure.process_tree_exit_verified => {
            ("failed", true, Some(format!("{:#}", failure.error)))
        }
        Err(failure) => (
            "cleanup-incomplete",
            false,
            Some(format!("{:#}", failure.error)),
        ),
    };
    write_conversion_journal(ConversionJournal {
        run_id,
        program: qemu_img,
        source,
        destination,
        state,
        pid: Some(child_pid),
        process_identity: child_identity.as_deref(),
        tree_exit_verified,
        error: error.as_deref(),
    })
    .map_err(|journal| {
        if tree_exit_verified {
            return StageSourceFailure::complete(journal);
        }
        StageSourceFailure::incomplete(journal)
    })?;
    result
}

fn write_conversion_journal(journal: ConversionJournal<'_>) -> Result<()> {
    let path = conversion_journal_path(journal.destination);
    let value = json!({
        "run_id": journal.run_id,
        "state": journal.state,
        "program": journal.program,
        "source": journal.source,
        "destination": journal.destination,
        "pid": journal.pid,
        "process_group": journal.pid,
        "process_identity": journal.process_identity,
        "tree_exit_verified": journal.tree_exit_verified,
        "error": journal.error,
    });
    let mut content = serde_json::to_vec_pretty(&value)
        .context("failed to encode qemu-img conversion journal")?;
    content.push(b'\n');
    qol_fs::atomic_write_durable(&path, &content)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub(super) fn conversion_journal_path(stage_path: &Path) -> PathBuf {
    stage_path.with_file_name("conversion.json")
}

fn terminate_conversion_process(
    child: &mut std::process::Child,
    process_tree: Option<&qol_process::ProcessTreeGuard>,
) -> Result<()> {
    let direct = qol_process::terminate_owned(child, PROCESS_SHUTDOWN_GRACE)
        .context("failed to stop qemu-img direct child");
    let reaped = child.wait().context("failed to reap qemu-img direct child");
    let tree = process_tree.map(|process_tree| {
        process_tree
            .terminate_and_wait(PROCESS_SHUTDOWN_GRACE)
            .map(|_proof| ())
            .context("qemu-img descendants survived cleanup")
    });
    let errors = [direct.err(), reaped.err(), tree.and_then(Result::err)]
        .into_iter()
        .flatten()
        .map(|error| format!("{error:#}"))
        .collect::<Vec<_>>();
    if errors.is_empty() {
        return Ok(());
    }
    bail!(errors.join("; "))
}

fn convert_args(source: &Path, destination: &Path) -> Vec<std::ffi::OsString> {
    vec![
        "convert".into(),
        "-f".into(),
        "qcow2".into(),
        "-O".into(),
        "qcow2".into(),
        "-S".into(),
        "4k".into(),
        source.as_os_str().to_os_string(),
        destination.as_os_str().to_os_string(),
    ]
}

fn verify_existing_image(staged: &StagedImage, cancelled: &impl Fn() -> bool) -> Result<()> {
    let metadata = fs::symlink_metadata(&staged.image_path)
        .with_context(|| format!("failed to inspect {}", staged.image_path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("managed image destination is not a regular file");
    }
    if metadata.len() != staged.size_bytes {
        bail!("managed image destination has a conflicting size");
    }
    if sha256_file(&staged.image_path, cancelled)? != staged.sha256 {
        bail!("managed image destination has a conflicting digest");
    }
    Ok(())
}

pub(super) fn sha256_file(path: &Path, mut cancelled: impl FnMut() -> bool) -> Result<String> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        if cancelled() {
            bail!("image hashing cancelled");
        }
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("failed to hash {}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn set_readonly(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("failed to make {} read-only", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::emu::image_import::tests::plan_fixture;

    #[test]
    fn conversion_uses_structured_sparse_qcow2_arguments() {
        let args = convert_args(Path::new("/source image"), Path::new("/stage image"));
        assert_eq!(
            args,
            [
                "convert",
                "-f",
                "qcow2",
                "-O",
                "qcow2",
                "-S",
                "4k",
                "/source image",
                "/stage image",
            ]
            .map(std::ffi::OsString::from)
        );
    }

    #[test]
    fn hashing_is_exact_and_cancellable() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.qcow2");
        fs::write(&source, b"image").unwrap();
        assert_eq!(
            sha256_file(&source, || false).unwrap(),
            "6105d6cc76af400325e94d588ce511be5bfdbb73b437dc51eca43917d7a43e3d"
        );
        assert!(sha256_file(&source, || true)
            .unwrap_err()
            .to_string()
            .contains("cancelled"));
    }

    #[test]
    fn concurrent_promotions_converge_on_one_immutable_content_path() {
        let root = tempfile::tempdir().unwrap();
        let content = b"verified image";
        let source = root.path().join("digest-source");
        fs::write(&source, content).unwrap();
        let digest = sha256_file(&source, || false).unwrap();
        let image_root = root.path().join("images");
        fs::create_dir_all(&image_root).unwrap();
        let image_path = qol_dev_env::managed_verified_image_path(&image_root, &digest).unwrap();
        let plans = ["run-a", "run-b"].map(|run_id| {
            let stage = root.path().join(format!("{run_id}.qcow2"));
            fs::write(&stage, content).unwrap();
            set_readonly(&stage).unwrap();
            let mut plan = plan_fixture(root.path());
            plan.run_id = run_id.to_string();
            plan.image_root = image_root.clone();
            (
                plan,
                StagedImage {
                    path: stage,
                    image_path: image_path.clone(),
                    sha256: digest.clone(),
                    size_bytes: content.len() as u64,
                    virtual_size: 1,
                },
            )
        });
        std::thread::scope(|scope| {
            for (plan, staged) in &plans {
                scope.spawn(move || promote_image(plan, staged, || false).unwrap());
            }
        });
        assert_eq!(fs::read(&image_path).unwrap(), content);
        assert!(fs::metadata(&image_path).unwrap().permissions().readonly());
        for (_, staged) in &plans {
            remove_promoted_stage(staged).unwrap();
        }
        assert!(plans.iter().all(|(_, staged)| !staged.path.exists()));
        assert!(fs::metadata(&image_path).unwrap().permissions().readonly());
    }

    #[test]
    fn cancellation_while_waiting_for_the_content_lock_never_publishes() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let root = tempfile::tempdir().unwrap();
        let content = b"verified image";
        let source = root.path().join("digest-source");
        fs::write(&source, content).unwrap();
        let digest = sha256_file(&source, || false).unwrap();
        let image_root = root.path().join("images");
        let image_path = qol_dev_env::managed_verified_image_path(&image_root, &digest).unwrap();
        let stage = root.path().join("stage.qcow2");
        fs::write(&stage, content).unwrap();
        set_readonly(&stage).unwrap();
        let mut plan = plan_fixture(root.path());
        plan.image_root = image_root;
        let staged = StagedImage {
            path: stage,
            image_path: image_path.clone(),
            sha256: digest.clone(),
            size_bytes: content.len() as u64,
            virtual_size: 1,
        };
        fs::create_dir_all(staged.image_path.parent().unwrap()).unwrap();
        let lock_path = staged
            .image_path
            .parent()
            .unwrap()
            .join(format!(".{digest}.lock"));
        let held = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)
            .unwrap();
        held.lock().unwrap();
        let polls = AtomicUsize::new(0);

        let error =
            promote_image(&plan, &staged, || polls.fetch_add(1, Ordering::SeqCst) > 0).unwrap_err();

        assert!(error.to_string().contains("cancelled"));
        assert!(!image_path.exists());
    }
}
