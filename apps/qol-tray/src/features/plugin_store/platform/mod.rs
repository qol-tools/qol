use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;

use super::release_assets::{PlatformTarget, SupportedArch, SupportedOs};

pub(super) trait PluginStorePlatformOps {
    fn host_os(&self) -> HostOs;
    fn lockfile_max_age(&self) -> Duration;
    fn lock_owner_alive(&self, pid: u32) -> Option<bool>;
    fn executable_permissions(&self, metadata: std::fs::Metadata) -> Option<std::fs::Permissions>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostOs {
    Linux,
    Macos,
    Windows,
    Unsupported(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostArch {
    X86_64,
    Aarch64,
    Unsupported(&'static str),
}

mod fallback_non_unix;
#[cfg(unix)]
mod fallback_unix;
const _: fallback_non_unix::Platform = fallback_non_unix::Platform;
#[cfg(unix)]
const _: fallback_unix::Platform = fallback_unix::Platform;
#[cfg(feature = "dev")]
mod dev;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(
    not(unix),
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
use fallback_non_unix as imp;
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "windows"))
))]
use fallback_unix as imp;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(feature = "dev")]
pub(super) use dev::{bind_public_runtime_socket, fixture_bundle_name};
pub(super) use imp::Platform;

impl HostOs {
    fn from_manifest_token(token: &'static str) -> Self {
        match token {
            "linux" => Self::Linux,
            "macos" => Self::Macos,
            "windows" => Self::Windows,
            other => Self::Unsupported(other),
        }
    }

    fn manifest_token(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
            Self::Unsupported(token) => token,
        }
    }

    fn display_label(self) -> &'static str {
        match self {
            Self::Linux => "Linux",
            Self::Macos => "macOS",
            Self::Windows => "Windows",
            Self::Unsupported(token) => token,
        }
    }

    fn executable_extension(self) -> &'static str {
        match self {
            Self::Windows => ".exe",
            Self::Linux | Self::Macos | Self::Unsupported(_) => "",
        }
    }
}

impl HostArch {
    fn current() -> Self {
        match std::env::consts::ARCH {
            "x86_64" => Self::X86_64,
            "aarch64" => Self::Aarch64,
            other => Self::Unsupported(other),
        }
    }

    fn manifest_token(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
            Self::Unsupported(token) => token,
        }
    }
}

pub(super) fn current_manifest_token() -> &'static str {
    Platform.host_os().manifest_token()
}

pub(super) fn display_label() -> &'static str {
    Platform.host_os().display_label()
}

pub(super) fn release_target() -> Result<PlatformTarget> {
    Ok(PlatformTarget {
        os: SupportedOs::from_token(Platform.host_os().manifest_token())?,
        arch: SupportedArch::from_token(HostArch::current().manifest_token())?,
    })
}

pub(super) fn lockfile_max_age() -> Duration {
    Platform.lockfile_max_age()
}

pub(super) fn lock_owner_alive(pid: u32) -> Option<bool> {
    Platform.lock_owner_alive(pid)
}

pub(super) fn dependency_binary_output_path(plugin_dir: &Path, binary_name: &str) -> PathBuf {
    dependency_binary_output_path_for(Platform.host_os(), plugin_dir, binary_name)
}

fn dependency_binary_output_path_for(
    host_os: HostOs,
    plugin_dir: &Path,
    binary_name: &str,
) -> PathBuf {
    let extension = host_os.executable_extension();
    if extension.is_empty() || Path::new(binary_name).extension().is_some() {
        return plugin_dir.join(binary_name);
    }
    plugin_dir.join(format!("{binary_name}{extension}"))
}

pub(super) fn built_binary_candidates(plugin_dir: &Path, binary_name: &str) -> Vec<PathBuf> {
    built_binary_candidates_for(Platform.host_os(), plugin_dir, binary_name)
}

fn built_binary_candidates_for(
    host_os: HostOs,
    plugin_dir: &Path,
    binary_name: &str,
) -> Vec<PathBuf> {
    let release_dir = plugin_dir.join("target").join("release");
    let unmodified = release_dir.join(binary_name);
    let resolved = dependency_binary_output_path_for(host_os, &release_dir, binary_name);
    if resolved == unmodified {
        return vec![unmodified];
    }
    vec![unmodified, resolved]
}

pub(super) fn executable_permissions(metadata: std::fs::Metadata) -> Option<std::fs::Permissions> {
    Platform.executable_permissions(metadata)
}

#[cfg(test)]
mod platform_tests;
