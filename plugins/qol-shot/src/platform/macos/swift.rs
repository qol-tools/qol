use anyhow::{anyhow, Context, Result};
use std::env;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const SWIFT_HELPER_CACHE_DIR: &str = "qol-shot-swift";
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
pub(super) const SWIFT_PRELUDE: &str = include_str!("../macos_swift/prelude.swift");
pub(super) const STATUS_OVERLAY_SWIFT: &str = include_str!("../macos_swift/status_overlay.swift");
pub(super) const RECORDING_OVERLAY_SWIFT: &str = include_str!("../macos_swift/recording_overlay.swift");
pub(super) const CLIPBOARD_WRITER_SWIFT: &str = include_str!("../macos_swift/clipboard_writer.swift");
pub(super) const VIDEO_COMPOSER_SWIFT: &str = include_str!("../macos_swift/video_composer.swift");
pub(super) const STATUS_OVERLAY_HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/status-overlay"));
pub(super) const RECORDING_OVERLAY_HELPER: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/recording-overlay"));
pub(super) const CLIPBOARD_WRITER_HELPER: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/clipboard-writer"));
pub(super) const VIDEO_COMPOSER_HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/video-composer"));

extern "C" {
    fn getuid() -> u32;
}

pub(super) fn prewarm_swift_helper(name: &'static str, body: &'static str, embedded_helper: &'static [u8]) {
    thread::spawn(move || {
        let _ = ensure_swift_helper(name, body, embedded_helper);
    });
}

pub(super) fn spawn_source_swift(body: &str, configure: impl FnOnce(&mut Command)) -> Result<Child> {
    let mut command = Command::new("swift");
    command.arg("-").stdin(Stdio::piped());
    configure(&mut command);
    let mut child = command.spawn().context("failed to start Swift source")?;

    let Some(stdin) = child.stdin.take() else {
        return Err(anyhow!("failed to open Swift source stdin"));
    };

    write_swift_source(stdin, body).context("failed to write Swift source")?;
    Ok(child)
}

fn write_swift_source(mut writer: impl Write, body: &str) -> std::io::Result<()> {
    writer.write_all(SWIFT_PRELUDE.as_bytes())?;
    writer.write_all(body.as_bytes())
}

pub(super) fn ensure_swift_helper(name: &str, body: &str, embedded_helper: &[u8]) -> Result<PathBuf> {
    let helper = swift_helper_path(name, body);
    if is_usable_swift_helper(&helper) {
        return Ok(helper);
    }

    let _ = fs::remove_file(&helper);
    install_embedded_swift_helper(name, embedded_helper, &helper)?;
    Ok(helper)
}

fn is_usable_swift_helper(path: &Path) -> bool {
    let Ok(metadata) = path.symlink_metadata() else {
        return false;
    };
    if !metadata.file_type().is_file() {
        return false;
    }
    metadata.permissions().mode() & 0o111 != 0 && metadata.uid() == current_uid()
}

fn install_embedded_swift_helper(name: &str, embedded_helper: &[u8], helper: &Path) -> Result<()> {
    let cache_dir = ensure_swift_helper_cache_dir()?;
    let token = format!("{}-{}", std::process::id(), unix_nanos());
    let temporary_helper = cache_dir.join(format!("{name}-{token}"));
    let mut file = File::create(&temporary_helper).context("failed to create Swift helper")?;
    file.write_all(embedded_helper)
        .context("failed to write embedded Swift helper")?;
    file.flush().context("failed to flush Swift helper")?;
    fs::set_permissions(&temporary_helper, fs::Permissions::from_mode(0o700))
        .context("failed to mark Swift helper executable")?;

    fs::rename(&temporary_helper, helper).context("failed to install embedded Swift helper")
}

fn ensure_swift_helper_cache_dir() -> Result<PathBuf> {
    let cache_dir = swift_helper_cache_dir();
    fs::create_dir_all(&cache_dir).context("failed to create Swift helper cache directory")?;
    let metadata = cache_dir
        .symlink_metadata()
        .context("failed to inspect Swift helper cache directory")?;
    if !metadata.file_type().is_dir() {
        return Err(anyhow!("Swift helper cache path is not a directory"));
    }
    fs::set_permissions(&cache_dir, fs::Permissions::from_mode(0o700))
        .context("failed to secure Swift helper cache directory")?;
    Ok(cache_dir)
}

fn swift_helper_cache_dir() -> PathBuf {
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home)
            .join("Library")
            .join("Caches")
            .join(SWIFT_HELPER_CACHE_DIR);
    }

    env::temp_dir().join(SWIFT_HELPER_CACHE_DIR)
}

fn swift_helper_path(name: &str, body: &str) -> PathBuf {
    swift_helper_cache_dir().join(format!("{name}-{:016x}", swift_source_hash(body)))
}

pub(super) fn swift_source_hash(body: &str) -> u64 {
    swift_source_hash_with_prelude(SWIFT_PRELUDE, body)
}

pub(super) fn swift_source_hash_with_prelude(prelude: &str, body: &str) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in prelude.bytes().chain(body.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn current_uid() -> u32 {
    unsafe { getuid() }
}

fn unix_nanos() -> u128 {
