use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use qol_dev_guest::{
    smbios_run_id, GuestHello, GuestSession, PROTOCOL_VERSION, WIN32_DISPLAY_PROTOCOL,
    WINDOWS_DEVICE_PATH, WINDOWS_IDENTITY_PATH,
};
use qol_headless::DoctorCheckResult;
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::SystemInformation::GetSystemFirmwareTable;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

mod virtio_port;

use super::GuestRunnerPlatform;
use crate::cli::RunOptions;
use qol_dev_guest::server::{parse_run_id, read_identity, read_run_id, serve_forever};

const RAW_SMBIOS_PROVIDER: u32 = u32::from_be_bytes(*b"RSMB");
const RAW_SMBIOS_HEADER_BYTES: usize = 8;
const WINDOWS_DESKTOP: &str = "windows";
const SERVICES_SESSION: u32 = 0;
const RUNNER_LOG_PATH: &str = r"C:\ProgramData\qol\guest-runner.log";
const RUNNER_LOG_MAX_BYTES: u64 = 1024 * 1024;

pub(super) struct Platform;

impl GuestRunnerPlatform for Platform {
    fn default_options(&self) -> RunOptions {
        RunOptions {
            device_path: PathBuf::from(WINDOWS_DEVICE_PATH),
            identity_path: PathBuf::from(WINDOWS_IDENTITY_PATH),
            run_id_path: None,
        }
    }

    fn run(&self, options: RunOptions) -> Result<()> {
        let image = read_identity(&options.identity_path)?;
        let run_id = match options.run_id_path {
            Some(path) => read_run_id(&path)?,
            None => firmware_run_id()?,
        };
        let hello = GuestHello {
            protocol_version: PROTOCOL_VERSION,
            run_id,
            image,
            session: current_session(),
            runner_pid: std::process::id(),
        };
        hello
            .validate_for(&hello.image.environment_id)
            .context("guest runner must start inside the prepared interactive session")?;
        serve_forever(
            || virtio_port::open(&options.device_path),
            &hello,
            |command| {
                command.creation_flags(CREATE_NO_WINDOW);
            },
            log_error,
        )
    }

    fn platform_check(&self) -> DoctorCheckResult {
        DoctorCheckResult::ok(
            "platform_supported",
            "Windows guest-control runtime is supported",
        )
    }

    fn runtime_paths_check(&self) -> DoctorCheckResult {
        let identity = PathBuf::from(WINDOWS_IDENTITY_PATH);
        let mut missing = Vec::new();
        if !identity.exists() {
            missing.push(format!("identity: {}", identity.display()));
        }
        if firmware_run_id().is_err() {
            missing.push("run id: SMBIOS OEM string".to_string());
        }
        if missing.is_empty() {
            return DoctorCheckResult::ok(
                "runtime_paths",
                "guest identity file and firmware run id are available",
            );
        }
        DoctorCheckResult::warn(
            "runtime_paths",
            format!(
                "prepared guest runtime inputs are missing: {}",
                missing.join(", ")
            ),
        )
        .with_fix("run qol-guest-runner inside a prepared qol development guest")
    }
}

fn log_error(error: &anyhow::Error) {
    let path = Path::new(RUNNER_LOG_PATH);
    let oversized = fs::metadata(path).is_ok_and(|metadata| metadata.len() > RUNNER_LOG_MAX_BYTES);
    let opened = OpenOptions::new()
        .create(true)
        .write(true)
        .append(!oversized)
        .truncate(oversized)
        .open(path);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    if let Ok(mut file) = opened {
        let _ = writeln!(file, "{millis} {error:#}");
    }
}

fn current_session() -> GuestSession {
    let interactive = interactive_session_id();
    GuestSession {
        user: env::var("USERNAME").unwrap_or_default(),
        desktop: interactive.map(|_| WINDOWS_DESKTOP.to_string()),
        session_type: Some(WIN32_DISPLAY_PROTOCOL.to_string()),
        display: interactive.map(|session| format!("session-{session}")),
        runtime_dir: None,
        dbus_session: false,
    }
}

fn interactive_session_id() -> Option<u32> {
    let mut session = 0;
    let ok = unsafe { ProcessIdToSessionId(std::process::id(), &mut session) };
    (ok != 0 && session != SERVICES_SESSION).then_some(session)
}

fn firmware_run_id() -> Result<String> {
    let table = raw_smbios_table()?;
    let run_id = smbios_run_id(&table).context("SMBIOS has no qol run id OEM string")?;
    parse_run_id(run_id.as_bytes())
}

fn raw_smbios_table() -> Result<Vec<u8>> {
    let size = unsafe { GetSystemFirmwareTable(RAW_SMBIOS_PROVIDER, 0, std::ptr::null_mut(), 0) };
    if size == 0 {
        bail!("Windows did not expose the raw SMBIOS table");
    }
    let mut buffer = vec![0_u8; size as usize];
    let written =
        unsafe { GetSystemFirmwareTable(RAW_SMBIOS_PROVIDER, 0, buffer.as_mut_ptr().cast(), size) };
    if written == 0 || written > size {
        bail!("failed to read the raw SMBIOS table");
    }
    buffer.truncate(written as usize);
    if buffer.len() < RAW_SMBIOS_HEADER_BYTES {
        bail!("raw SMBIOS table is truncated");
    }
    let declared = u32::from_le_bytes(buffer[4..8].try_into()?) as usize;
    let end = RAW_SMBIOS_HEADER_BYTES
        .checked_add(declared)
        .filter(|end| *end <= buffer.len())
        .context("raw SMBIOS table length is out of range")?;
    Ok(buffer[RAW_SMBIOS_HEADER_BYTES..end].to_vec())
}
