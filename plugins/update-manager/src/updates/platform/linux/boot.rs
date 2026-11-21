use std::fs;
use std::time::SystemTime;

use chrono::{DateTime, Duration, Local, NaiveDateTime};

use super::names::two_parts;
use crate::platform::linux::command::lenient;
use crate::platform::linux::packages::{
    installed_images, kernel_abi, running_kernel, running_release, version_key,
};
use crate::updates::{when, Finish};

const BOOT_ID: &str = "/proc/sys/kernel/random/boot_id";
const NVIDIA: &str = "/sys/module/nvidia/version";
const REBOOT_PKGS: &str = "/run/reboot-required.pkgs";
const DPKG_INFO: &str = "/var/lib/dpkg/info";

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Running {
    pub(super) kernel: Option<String>,
    pub(super) nvidia: Option<String>,
}

pub(super) fn running() -> Running {
    Running {
        kernel: running_kernel(),
        nvidia: fs::read_to_string(NVIDIA)
            .ok()
            .map(|version| two_parts(version.trim())),
    }
}

pub(super) fn boot_id() -> Option<String> {
    fs::read_to_string(BOOT_ID)
        .ok()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
}

pub(super) fn boot_time(now: NaiveDateTime) -> Option<NaiveDateTime> {
    let uptime = fs::read_to_string("/proc/uptime").ok()?;
    let seconds: f64 = uptime.split_whitespace().next()?.parse().ok()?;
    Some(now - Duration::seconds(seconds as i64))
}

pub(super) fn finish(running: &Running, now: NaiveDateTime) -> Vec<Finish> {
    let mut finish = Vec::new();
    let flavor = running_release().and_then(|release| {
        let abi = kernel_abi(&format!("linux-image-{release}"))?;
        release.strip_prefix(&format!("{abi}-")).map(String::from)
    });
    let images = installed_images().unwrap_or_default();
    if let (Some(current), Some(flavor)) = (running.kernel.as_deref(), flavor.as_deref()) {
        if let Some((newest, package)) = newest_kernel(&images, flavor)
            .filter(|(newest, _)| version_key(newest) > version_key(current))
        {
            let installed = modified(&format!("{DPKG_INFO}/{package}.list"))
                .map(|time| format!(" {}", when(time, now)))
                .unwrap_or_default();
            let label = format!("Linux kernel {newest}");
            finish.push(Finish {
                id: "kernel".into(),
                label: label.clone(),
                name: label,
                finish: format!("Installed{installed}. Until a restart, {current} keeps running."),
            });
        }
    }
    let drivers = lenient(
        "dpkg-query",
        &[
            "-W",
            "-f",
            "${Package}\t${db:Status-Abbrev}\t${Version}\n",
            "nvidia-driver-*",
        ],
    );
    if let (Some(loaded), Some(installed)) = (running.nvidia.as_deref(), installed_driver(&drivers))
    {
        if installed != loaded {
            let label = format!("NVIDIA driver {installed}");
            finish.push(Finish {
                id: "nvidia".into(),
                label: label.clone(),
                name: label,
                finish: format!("Installed. Until a restart, {loaded} keeps running."),
            });
        }
    }
    for package in fs::read_to_string(REBOOT_PKGS).unwrap_or_default().lines() {
        let package = package.trim();
        if package.is_empty()
            || package.starts_with("linux-")
            || package.contains("nvidia")
            || finish
                .iter()
                .any(|row| row.id == format!("package:{package}"))
        {
            continue;
        }
        finish.push(Finish {
            id: format!("package:{package}"),
            label: package.to_string(),
            name: package.to_string(),
            finish: "Installed. It finishes at the restart.".into(),
        });
    }
    finish
}

fn modified(path: &str) -> Option<NaiveDateTime> {
    let time: SystemTime = fs::metadata(path).ok()?.modified().ok()?;
    Some(DateTime::<Local>::from(time).naive_local())
}

pub(super) fn newest_kernel(images: &[String], flavor: &str) -> Option<(String, String)> {
    images
        .iter()
        .filter(|package| package.ends_with(&format!("-{flavor}")))
        .filter_map(|package| Some((kernel_abi(package)?, package.clone())))
        .max_by_key(|(abi, _)| version_key(abi))
}

pub(super) fn installed_driver(listing: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let mut fields = line.split('\t');
        let package = fields.next()?;
        let status = fields.next()?;
        let version = fields.next()?;
        let branch = package.strip_prefix("nvidia-driver-")?;
        (status.trim() == "ii" && branch.chars().next()?.is_ascii_digit())
            .then(|| two_parts(version))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_installed_kernel_of_the_running_flavor_wins() {
        let images: Vec<String> = [
            "linux-image-6.8.0-146-lowlatency",
            "linux-image-6.8.0-147-lowlatency",
            "linux-image-6.8.0-148-generic",
            "linux-image-unsigned-6.8.0-150-generic",
            "linux-image-lowlatency",
        ]
        .map(String::from)
        .to_vec();
        let newest = |flavor| newest_kernel(&images, flavor);
        assert_eq!(
            newest("lowlatency"),
            Some((
                "6.8.0-147".to_string(),
                "linux-image-6.8.0-147-lowlatency".to_string()
            ))
        );
        assert_eq!(
            newest("generic"),
            Some((
                "6.8.0-150".to_string(),
                "linux-image-unsigned-6.8.0-150-generic".to_string()
            )),
            "an unsigned image keeps its own package name, so its install time is found"
        );
    }

    #[test]
    fn the_installed_driver_reads_its_branch_version() {
        let listing = "nvidia-driver-580\tii \t580.178.04-0ubuntu0.24.04.1\n\
                       nvidia-driver-570\tun \t\n";
        assert_eq!(installed_driver(listing).as_deref(), Some("580.178"));
        assert_eq!(
            installed_driver("nvidia-driver-assistant\tii \t0.1\n"),
            None
        );
    }
}
