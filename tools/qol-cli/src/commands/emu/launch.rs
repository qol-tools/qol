use anyhow::{anyhow, bail, Result};
use qol_dev_env::resources::ParentLeaseClaim;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::guest::GuestAdapter;
use super::{AccelerationRequirement, BackendImageKind, Firmware, GuestArch};
use crate::commands::dev_env::resources::{
    ResourceProfile, MAX_CPUS, MAX_MEMORY_MB, MIN_CPUS, MIN_MEMORY_MB,
};

const DEFAULT_MEMORY_MB: u32 = 4096;
const DEFAULT_CPUS: u16 = 2;
const MAX_RUN_ID_LEN: usize = 64;
const MAX_ENVIRONMENT_ID_LEN: usize = 255;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DisplayMode {
    Host,
    #[default]
    None,
}

impl DisplayMode {
    pub(crate) fn qemu_value(self, host_display: &str) -> &str {
        match self {
            Self::Host => host_display,
            Self::None => "none",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LaunchOptions {
    pub(crate) target: String,
    pub(crate) environment_id: Option<String>,
    pub(crate) display: DisplayMode,
    pub(crate) offline: bool,
    pub(crate) memory_mb: u32,
    pub(crate) cpus: u16,
    pub(crate) run_id: Option<String>,
    pub(crate) parent_lease: Option<ParentLeaseClaim>,
    pub(crate) guest_adapter: Option<GuestAdapter>,
    pub(crate) guest_image_revision: Option<String>,
    pub(crate) payload_manifest: Option<PathBuf>,
    pub(crate) payload_image: Option<PathBuf>,
    pub(crate) run_root: Option<PathBuf>,
    pub(crate) image_kind: Option<BackendImageKind>,
    pub(crate) acceleration: AccelerationRequirement,
    pub(crate) arch: Option<GuestArch>,
    pub(crate) firmware: Option<Firmware>,
    pub(crate) usb_host: Option<PathBuf>,
    pub(crate) worktree: Option<PathBuf>,
    pub(crate) image_import_config: Option<PathBuf>,
}

impl LaunchOptions {
    pub(crate) fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            environment_id: None,
            display: DisplayMode::None,
            offline: false,
            memory_mb: DEFAULT_MEMORY_MB,
            cpus: DEFAULT_CPUS,
            run_id: None,
            parent_lease: None,
            guest_adapter: None,
            guest_image_revision: None,
            payload_manifest: None,
            payload_image: None,
            run_root: None,
            image_kind: None,
            acceleration: AccelerationRequirement::AllowTcg,
            arch: None,
            firmware: None,
            usb_host: None,
            worktree: None,
            image_import_config: None,
        }
    }

    pub(crate) fn qemu_args(&self, host_display: &str) -> Vec<String> {
        vec![
            "-m".to_string(),
            self.memory_mb.to_string(),
            "-smp".to_string(),
            self.cpus.to_string(),
            "-display".to_string(),
            self.display.qemu_value(host_display).to_string(),
        ]
    }

    pub(crate) fn validate_payload_isolation(&self) -> Result<()> {
        validate_payload_isolation(
            self.payload_manifest.as_deref(),
            self.payload_image.as_deref(),
            self.display,
            self.offline,
            self.parent_lease.is_some(),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildOperation<'a> {
    Up,
    Run(&'a str),
}

pub(crate) struct ChildLaunch<'a> {
    pub(crate) operation: ChildOperation<'a>,
    pub(crate) target: &'a Path,
    pub(crate) environment_id: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) parent_lease: &'a ParentLeaseClaim,
    pub(crate) guest_adapter: Option<GuestAdapter>,
    pub(crate) guest_image_revision: Option<&'a str>,
    pub(crate) payload_manifest: Option<&'a Path>,
    pub(crate) payload_image: Option<&'a Path>,
    pub(crate) run_root: Option<&'a Path>,
    pub(crate) image_kind: Option<&'a str>,
    pub(crate) display: DisplayMode,
    pub(crate) offline: bool,
    pub(crate) resources: ResourceProfile,
    pub(crate) acceleration: Option<&'a str>,
    pub(crate) arch: Option<&'a str>,
    pub(crate) firmware: Option<&'a str>,
    pub(crate) usb_host: Option<&'a Path>,
}

pub(crate) fn child_args(launch: ChildLaunch<'_>) -> Result<Vec<OsString>> {
    validate_run_id(launch.run_id)?;
    validate_environment_id(launch.environment_id)?;
    validate_payload_isolation(
        launch.payload_manifest,
        launch.payload_image,
        launch.display,
        launch.offline,
        true,
    )?;
    let mut args = vec![OsString::from("emu")];
    match launch.operation {
        ChildOperation::Up => args.push(OsString::from("up")),
        ChildOperation::Run(workflow) => {
            if workflow.is_empty() {
                bail!("workflow id must not be empty");
            }
            args.push(OsString::from("run"));
            args.push(OsString::from(workflow));
        }
    }
    args.push(launch.target.as_os_str().to_os_string());
    args.push(OsString::from(match launch.display {
        DisplayMode::Host => "--windowed",
        DisplayMode::None => "--headless",
    }));
    if launch.offline {
        args.push(OsString::from("--offline"));
    }
    args.extend([
        OsString::from("--memory-mb"),
        OsString::from(launch.resources.memory_mb.to_string()),
        OsString::from("--cpus"),
        OsString::from(launch.resources.cpus.to_string()),
        OsString::from("--run-id"),
        OsString::from(launch.run_id),
        OsString::from("--parent-lease"),
        OsString::from(launch.parent_lease.as_str()),
        OsString::from("--environment-id"),
        OsString::from(launch.environment_id),
    ]);
    if let Some(adapter) = launch.guest_adapter {
        args.extend([
            OsString::from("--guest-adapter"),
            OsString::from(adapter.as_str()),
        ]);
    }
    if let Some(revision) = launch.guest_image_revision {
        validate_safe_token(revision, "--guest-image-revision")?;
        args.extend([
            OsString::from("--guest-image-revision"),
            OsString::from(revision),
        ]);
    }
    if let Some(payload_manifest) = launch.payload_manifest {
        append_absolute_path_option(&mut args, "--payload-manifest", payload_manifest)?;
    }
    if let Some(payload_image) = launch.payload_image {
        append_absolute_path_option(&mut args, "--payload-image", payload_image)?;
    }
    if let Some(run_root) = launch.run_root {
        args.extend([
            OsString::from("--run-root"),
            run_root.as_os_str().to_os_string(),
        ]);
    }
    if let Some(image_kind) = launch.image_kind {
        let image_kind = BackendImageKind::parse(image_kind)
            .ok_or_else(|| anyhow!("--image-kind must be one of: qcow2, raw, img, iso"))?;
        args.extend([
            OsString::from("--image-kind"),
            OsString::from(image_kind.as_str()),
        ]);
    }
    if let Some(acceleration) = launch.acceleration {
        let acceleration = AccelerationRequirement::parse(acceleration)
            .ok_or_else(|| anyhow!("--acceleration must be one of: hardware, allow-tcg"))?;
        args.extend([
            OsString::from("--acceleration"),
            OsString::from(acceleration.as_str()),
        ]);
    }
    if let Some(arch) = launch.arch {
        let arch = GuestArch::parse(arch)
            .ok_or_else(|| anyhow!("--arch must be one of: x86_64, aarch64"))?;
        args.extend([OsString::from("--arch"), OsString::from(arch.as_str())]);
    }
    if let Some(firmware) = launch.firmware {
        let firmware = Firmware::parse(firmware)
            .ok_or_else(|| anyhow!("--firmware must be one of: bios, uefi"))?;
        args.extend([
            OsString::from("--firmware"),
            OsString::from(firmware.as_str()),
        ]);
    }
    if let Some(usb_host) = launch.usb_host {
        append_absolute_path_option(&mut args, "--usb-host", usb_host)?;
    }
    Ok(args)
}

fn append_absolute_path_option(args: &mut Vec<OsString>, option: &str, path: &Path) -> Result<()> {
    if !path.is_absolute() {
        bail!("{option} must be an absolute path");
    }
    args.extend([OsString::from(option), path.as_os_str().to_os_string()]);
    Ok(())
}

pub(crate) fn parse_launch_options(args: &[OsString], usage: &str) -> Result<LaunchOptions> {
    let mut target = None;
    let mut environment_id = None;
    let mut display = None;
    let mut offline = None;
    let mut memory_mb = None;
    let mut cpus = None;
    let mut run_id = None;
    let mut parent_lease = None;
    let mut guest_adapter = None;
    let mut guest_image_revision = None;
    let mut payload_manifest = None;
    let mut payload_image = None;
    let mut run_root = None;
    let mut image_kind = None;
    let mut acceleration = None;
    let mut arch = None;
    let mut firmware = None;
    let mut usb_host = None;
    let mut index = 0;

    while index < args.len() {
        let argument = utf8_arg(&args[index])?;
        match argument {
            "--headless" => {
                reject_duplicate(display.is_some(), "--headless")?;
                display = Some(DisplayMode::None);
                index += 1;
            }
            "--windowed" => {
                reject_duplicate(display.is_some(), "--windowed")?;
                display = Some(DisplayMode::Host);
                index += 1;
            }
            "--offline" => {
                reject_duplicate(offline.is_some(), "--offline")?;
                offline = Some(true);
                index += 1;
            }
            "--memory-mb" => {
                reject_duplicate(memory_mb.is_some(), "--memory-mb")?;
                let value = option_value(args, index, "--memory-mb")?;
                memory_mb =
                    Some(parse_bounded(value, "--memory-mb", MIN_MEMORY_MB, MAX_MEMORY_MB)? as u32);
                index += 2;
            }
            "--cpus" => {
                reject_duplicate(cpus.is_some(), "--cpus")?;
                let value = option_value(args, index, "--cpus")?;
                cpus = Some(parse_bounded(value, "--cpus", MIN_CPUS, MAX_CPUS)? as u16);
                index += 2;
            }
            "--run-id" => {
                reject_duplicate(run_id.is_some(), "--run-id")?;
                let value = option_value(args, index, "--run-id")?;
                validate_run_id(value)?;
                run_id = Some(value.to_string());
                index += 2;
            }
            "--parent-lease" => {
                reject_duplicate(parent_lease.is_some(), "--parent-lease")?;
                let value = option_value(args, index, "--parent-lease")?;
                parent_lease = Some(ParentLeaseClaim::parse(value)?);
                index += 2;
            }
            "--guest-adapter" => {
                reject_duplicate(guest_adapter.is_some(), "--guest-adapter")?;
                let value = option_value(args, index, "--guest-adapter")?;
                guest_adapter = Some(GuestAdapter::parse(value).ok_or_else(|| {
                    anyhow!(
                        "--guest-adapter must be one of: debian-nocloud, macos-desktop, mint-cinnamon, windows-desktop"
                    )
                })?);
                index += 2;
            }
            "--guest-image-revision" => {
                reject_duplicate(guest_image_revision.is_some(), "--guest-image-revision")?;
                let value = option_value(args, index, "--guest-image-revision")?;
                validate_safe_token(value, "--guest-image-revision")?;
                guest_image_revision = Some(value.to_string());
                index += 2;
            }
            "--payload-manifest" => {
                parse_path_option(args, &mut payload_manifest, index, "--payload-manifest")?;
                index += 2;
            }
            "--payload-image" => {
                parse_path_option(args, &mut payload_image, index, "--payload-image")?;
                index += 2;
            }
            "--environment-id" => {
                reject_duplicate(environment_id.is_some(), "--environment-id")?;
                let value = option_value(args, index, "--environment-id")?;
                validate_environment_id(value)?;
                environment_id = Some(value.to_string());
                index += 2;
            }
            "--run-root" => {
                reject_duplicate(run_root.is_some(), "--run-root")?;
                run_root = Some(PathBuf::from(option_value(args, index, "--run-root")?));
                index += 2;
            }
            "--image-kind" => {
                reject_duplicate(image_kind.is_some(), "--image-kind")?;
                let value = option_value(args, index, "--image-kind")?;
                image_kind =
                    Some(BackendImageKind::parse(value).ok_or_else(|| {
                        anyhow!("--image-kind must be one of: qcow2, raw, img, iso")
                    })?);
                index += 2;
            }
            "--acceleration" => {
                reject_duplicate(acceleration.is_some(), "--acceleration")?;
                let value = option_value(args, index, "--acceleration")?;
                acceleration = Some(AccelerationRequirement::parse(value).ok_or_else(|| {
                    anyhow!("--acceleration must be one of: hardware, allow-tcg")
                })?);
                index += 2;
            }
            "--arch" => {
                reject_duplicate(arch.is_some(), "--arch")?;
                let value = option_value(args, index, "--arch")?;
                arch = Some(
                    GuestArch::parse(value)
                        .ok_or_else(|| anyhow!("--arch must be one of: x86_64, aarch64"))?,
                );
                index += 2;
            }
            "--firmware" => {
                reject_duplicate(firmware.is_some(), "--firmware")?;
                let value = option_value(args, index, "--firmware")?;
                firmware = Some(
                    Firmware::parse(value)
                        .ok_or_else(|| anyhow!("--firmware must be one of: bios, uefi"))?,
                );
                index += 2;
            }
            "--usb-host" => {
                parse_path_option(args, &mut usb_host, index, "--usb-host")?;
                index += 2;
            }
            option if option.starts_with('-') => bail!("unknown launch option `{option}`"),
            value => {
                if value.is_empty() || target.is_some() {
                    bail!("usage: {usage}");
                }
                target = Some(value.to_string());
                index += 1;
            }
        }
    }

    let target = target.ok_or_else(|| anyhow!("usage: {usage}"))?;
    let mut options = LaunchOptions::new(target);
    options.environment_id = environment_id;
    options.display = display.unwrap_or_default();
    options.offline = offline.unwrap_or(false);
    options.memory_mb = memory_mb.unwrap_or(DEFAULT_MEMORY_MB);
    options.cpus = cpus.unwrap_or(DEFAULT_CPUS);
    options.run_id = run_id;
    options.parent_lease = parent_lease;
    options.guest_adapter = guest_adapter;
    options.guest_image_revision = guest_image_revision;
    options.payload_manifest = payload_manifest;
    options.payload_image = payload_image;
    options.run_root = run_root;
    options.image_kind = image_kind;
    options.acceleration = acceleration.unwrap_or(AccelerationRequirement::AllowTcg);
    options.arch = arch;
    options.firmware = firmware;
    options.usb_host = usb_host;
    options.validate_payload_isolation()?;
    Ok(options)
}

fn validate_payload_isolation(
    payload_manifest: Option<&Path>,
    payload_image: Option<&Path>,
    display: DisplayMode,
    offline: bool,
    parent_covered: bool,
) -> Result<()> {
    match (payload_manifest, payload_image) {
        (None, None) => return Ok(()),
        (Some(_), Some(_)) => {}
        _ => bail!("--payload-manifest and --payload-image must be provided together"),
    }
    if !parent_covered {
        bail!("payload transport requires a parent-covered environment or flow launch");
    }
    if display != DisplayMode::None {
        bail!("parent-covered payload launches must be headless");
    }
    if !offline {
        bail!("parent-covered payload launches must be offline");
    }
    Ok(())
}

fn parse_path_option(
    args: &[OsString],
    slot: &mut Option<PathBuf>,
    index: usize,
    option: &str,
) -> Result<()> {
    reject_duplicate(slot.is_some(), option)?;
    let path = PathBuf::from(option_value(args, index, option)?);
    if !path.is_absolute() {
        bail!("{option} must be an absolute path");
    }
    *slot = Some(path);
    Ok(())
}

fn utf8_arg(argument: &OsString) -> Result<&str> {
    argument
        .to_str()
        .ok_or_else(|| anyhow!("launch argument is not valid UTF-8"))
}

fn option_value<'a>(args: &'a [OsString], index: usize, option: &str) -> Result<&'a str> {
    let value = args
        .get(index + 1)
        .ok_or_else(|| anyhow!("{option} requires a value"))?;
    let value = utf8_arg(value)?;
    if value.starts_with('-') {
        bail!("{option} requires a value");
    }
    Ok(value)
}

fn reject_duplicate(duplicate: bool, option: &str) -> Result<()> {
    if duplicate {
        bail!("duplicate launch option `{option}`");
    }
    Ok(())
}

fn parse_bounded(value: &str, option: &str, minimum: u64, maximum: u64) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("{option} must be an integer from {minimum} to {maximum}");
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|_| anyhow!("{option} must be an integer from {minimum} to {maximum}"))?;
    if !(minimum..=maximum).contains(&parsed) {
        bail!("{option} must be from {minimum} to {maximum}");
    }
    Ok(parsed)
}

fn validate_run_id(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_RUN_ID_LEN || !safe_id_segment(value) {
        bail!("--run-id must contain 1 to {MAX_RUN_ID_LEN} ASCII letters, digits, hyphens, or underscores");
    }
    Ok(())
}

fn validate_environment_id(value: &str) -> Result<()> {
    let safe = !value.is_empty()
        && value.len() <= MAX_ENVIRONMENT_ID_LEN
        && value.split('/').all(safe_id_segment);
    if !safe {
        bail!("--environment-id must contain nonempty slash-separated ASCII id segments");
    }
    Ok(())
}

fn validate_safe_token(value: &str, option: &str) -> Result<()> {
    let mut bytes = value.bytes();
    let safe = value.len() <= 128
        && bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if !safe {
        bail!("{option} must be a safe nonempty token of at most 128 bytes");
    }
    Ok(())
}

fn safe_id_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    fn parse(values: &[&str]) -> Result<LaunchOptions> {
        parse_launch_options(&argv(values), "qol emu up <environment> [options]")
    }

    fn child_launch<'a>(
        operation: ChildOperation<'a>,
        parent_lease: &'a ParentLeaseClaim,
    ) -> ChildLaunch<'a> {
        ChildLaunch {
            operation,
            target: Path::new("/images/debian.qcow2"),
            environment_id: "linux/debian",
            run_id: "debian-lane-1",
            parent_lease,
            guest_adapter: Some(GuestAdapter::DebianNocloud),
            guest_image_revision: Some("debian-12-qol-1"),
            payload_manifest: Some(Path::new("/runs/flow/payload/manifest.json")),
            payload_image: Some(Path::new("/runs/flow/payload.iso")),
            run_root: Some(Path::new("/runs/cases")),
            image_kind: Some("qcow2"),
            display: DisplayMode::None,
            offline: true,
            resources: ResourceProfile {
                memory_mb: 768,
                cpus: 2,
            },
            acceleration: Some("hardware"),
            arch: Some("x86_64"),
            firmware: Some("bios"),
            usb_host: None,
        }
    }

    #[test]
    fn child_launch_args_are_shared_by_env_and_flow() {
        let parent_lease = ParentLeaseClaim::parse("debian-batch-1").unwrap();
        let up = child_args(child_launch(ChildOperation::Up, &parent_lease)).unwrap();
        assert_eq!(
            up,
            argv(&[
                "emu",
                "up",
                "/images/debian.qcow2",
                "--headless",
                "--offline",
                "--memory-mb",
                "768",
                "--cpus",
                "2",
                "--run-id",
                "debian-lane-1",
                "--parent-lease",
                "debian-batch-1",
                "--environment-id",
                "linux/debian",
                "--guest-adapter",
                "debian-nocloud",
                "--guest-image-revision",
                "debian-12-qol-1",
                "--payload-manifest",
                "/runs/flow/payload/manifest.json",
                "--payload-image",
                "/runs/flow/payload.iso",
                "--run-root",
                "/runs/cases",
                "--image-kind",
                "qcow2",
                "--acceleration",
                "hardware",
                "--arch",
                "x86_64",
                "--firmware",
                "bios",
            ])
        );

        let run = child_args(child_launch(
            ChildOperation::Run("leaves-no-trace"),
            &parent_lease,
        ))
        .unwrap();
        assert_eq!(&run[..3], &argv(&["emu", "run", "leaves-no-trace"]));
        assert_eq!(&run[3..], &up[2..]);
    }

    #[test]
    fn child_launch_reuses_identity_and_manifest_validation() {
        let parent_lease = ParentLeaseClaim::parse("debian-batch-1").unwrap();
        let mut cases = [
            child_launch(ChildOperation::Up, &parent_lease),
            child_launch(ChildOperation::Up, &parent_lease),
            child_launch(ChildOperation::Run(""), &parent_lease),
            child_launch(ChildOperation::Up, &parent_lease),
        ];
        cases[0].run_id = "bad/run";
        cases[1].environment_id = "../bad";
        cases[2].arch = None;
        cases[3].firmware = Some("efi");
        for launch in cases {
            assert!(child_args(launch).is_err());
        }

        let mut invalid_kind = child_launch(ChildOperation::Up, &parent_lease);
        invalid_kind.image_kind = Some("vhdx");
        assert!(child_args(invalid_kind).is_err());

        let mut invalid_acceleration = child_launch(ChildOperation::Up, &parent_lease);
        invalid_acceleration.acceleration = Some("kvm");
        assert!(child_args(invalid_acceleration).is_err());
    }

    #[test]
    fn child_payload_launches_require_headless_offline_isolation() {
        let parent_lease = ParentLeaseClaim::parse("debian-batch-1").unwrap();
        let mut windowed = child_launch(ChildOperation::Run("desktop"), &parent_lease);
        windowed.display = DisplayMode::Host;
        assert_eq!(
            child_args(windowed).unwrap_err().to_string(),
            "parent-covered payload launches must be headless"
        );

        let mut online = child_launch(ChildOperation::Run("desktop"), &parent_lease);
        online.offline = false;
        assert_eq!(
            child_args(online).unwrap_err().to_string(),
            "parent-covered payload launches must be offline"
        );
    }

    #[test]
    fn parses_defaults_and_options_in_any_position() {
        let default = parse(&["mint"]).unwrap();
        assert_eq!(default, LaunchOptions::new("mint"));

        let cases = [
            vec![
                "mint",
                "--headless",
                "--offline",
                "--memory-mb",
                "8192",
                "--cpus",
                "8",
                "--run-id",
                "lane_01",
                "--parent-lease",
                "parent-flow",
                "--guest-adapter",
                "debian-nocloud",
                "--guest-image-revision",
                "mint-22.3-qol-1",
                "--payload-manifest",
                "/runs/flow/payload/manifest.json",
                "--payload-image",
                "/runs/flow/payload.iso",
                "--environment-id",
                "linux/mint",
                "--run-root",
                "/runs/mint",
                "--image-kind",
                "qcow2",
                "--acceleration",
                "hardware",
                "--arch",
                "x86_64",
                "--firmware",
                "uefi",
            ],
            vec![
                "--environment-id",
                "linux/mint",
                "--guest-adapter",
                "debian-nocloud",
                "--guest-image-revision",
                "mint-22.3-qol-1",
                "--payload-image",
                "/runs/flow/payload.iso",
                "--payload-manifest",
                "/runs/flow/payload/manifest.json",
                "--parent-lease",
                "parent-flow",
                "--run-id",
                "lane_01",
                "--run-root",
                "/runs/mint",
                "--acceleration",
                "hardware",
                "--image-kind",
                "qcow2",
                "--firmware",
                "uefi",
                "--cpus",
                "8",
                "--arch",
                "x86_64",
                "mint",
                "--memory-mb",
                "8192",
                "--headless",
                "--offline",
            ],
        ];
        for values in cases {
            let parsed = parse(&values).unwrap();
            assert_eq!(parsed.target, "mint", "values: {values:?}");
            assert_eq!(parsed.environment_id.as_deref(), Some("linux/mint"));
            assert_eq!(parsed.display, DisplayMode::None);
            assert!(parsed.offline);
            assert_eq!(parsed.memory_mb, 8192);
            assert_eq!(parsed.cpus, 8);
            assert_eq!(parsed.run_id.as_deref(), Some("lane_01"));
            assert_eq!(
                parsed.parent_lease.as_ref().unwrap().as_str(),
                "parent-flow"
            );
            assert_eq!(parsed.guest_adapter, Some(GuestAdapter::DebianNocloud));
            assert_eq!(
                parsed.guest_image_revision.as_deref(),
                Some("mint-22.3-qol-1")
            );
            assert_eq!(
                parsed.payload_manifest.as_deref(),
                Some(Path::new("/runs/flow/payload/manifest.json"))
            );
            assert_eq!(
                parsed.payload_image.as_deref(),
                Some(Path::new("/runs/flow/payload.iso"))
            );
            assert_eq!(parsed.run_root.as_deref(), Some(Path::new("/runs/mint")));
            assert_eq!(parsed.image_kind, Some(BackendImageKind::Qcow2));
            assert_eq!(parsed.acceleration, AccelerationRequirement::Hardware);
            assert_eq!(parsed.arch, Some(GuestArch::X86_64));
            assert_eq!(parsed.firmware, Some(Firmware::Uefi));
        }
    }

    #[test]
    fn parses_and_forwards_an_explicit_usb_host_device() {
        let parsed = parse(&["mint", "--usb-host", "/dev/bus/usb/001/007"]).unwrap();
        assert_eq!(
            parsed.usb_host.as_deref(),
            Some(Path::new("/dev/bus/usb/001/007"))
        );

        let parent_lease = ParentLeaseClaim::parse("debian-batch-1").unwrap();
        let mut launch = child_launch(ChildOperation::Up, &parent_lease);
        launch.usb_host = Some(Path::new("/dev/bus/usb/001/007"));
        let args = child_args(launch).unwrap();
        assert_eq!(args[args.len() - 2], "--usb-host");
        assert_eq!(args[args.len() - 1], "/dev/bus/usb/001/007");
    }

    #[test]
    fn parser_rejects_payload_launches_that_can_touch_the_host_session_or_network() {
        let payload = [
            "mint",
            "--parent-lease",
            "parent-flow",
            "--payload-manifest",
            "/runs/flow/payload/manifest.json",
            "--payload-image",
            "/runs/flow/payload.iso",
        ];
        assert_eq!(
            parse(&payload).unwrap_err().to_string(),
            "parent-covered payload launches must be offline"
        );

        let mut windowed = payload.to_vec();
        windowed.extend(["--offline", "--windowed"]);
        assert_eq!(
            parse(&windowed).unwrap_err().to_string(),
            "parent-covered payload launches must be headless"
        );

        let mut isolated = payload.to_vec();
        isolated.extend(["--offline", "--headless"]);
        assert!(parse(&isolated).is_ok());
    }

    #[test]
    fn produces_qemu_values_for_host_and_headless_modes() {
        let mut host = LaunchOptions::new("mint");
        host.display = DisplayMode::Host;
        assert_eq!(
            host.qemu_args("gtk,zoom-to-fit=on"),
            strings(&["-m", "4096", "-smp", "2", "-display", "gtk,zoom-to-fit=on"])
        );

        let mut headless = LaunchOptions::new("mint");
        headless.display = DisplayMode::None;
        headless.memory_mb = 512;
        headless.cpus = 1;
        assert_eq!(
            headless.qemu_args("gtk"),
            strings(&["-m", "512", "-smp", "1", "-display", "none"])
        );
    }

    #[test]
    fn enforces_numeric_syntax_and_bounds() {
        let valid = [
            ("--memory-mb", "256"),
            ("--memory-mb", "1048576"),
            ("--cpus", "1"),
            ("--cpus", "256"),
        ];
        for (option, value) in valid {
            assert!(parse(&["mint", option, value]).is_ok(), "{option} {value}");
        }

        let invalid = [
            ("--memory-mb", "0"),
            ("--memory-mb", "255"),
            ("--memory-mb", "1048577"),
            ("--memory-mb", "18446744073709551616"),
            ("--memory-mb", "4GiB"),
            ("--cpus", "0"),
            ("--cpus", "257"),
            ("--cpus", "+2"),
            ("--cpus", "2.0"),
            ("--cpus", "２"),
        ];
        for (option, value) in invalid {
            assert!(parse(&["mint", option, value]).is_err(), "{option} {value}");
        }
    }

    #[test]
    fn rejects_duplicate_unknown_and_missing_options() {
        let invalid = [
            vec!["mint", "--headless", "--headless"],
            vec!["mint", "--windowed", "--windowed"],
            vec!["mint", "--headless", "--windowed"],
            vec!["mint", "--offline", "--offline"],
            vec!["mint", "--memory-mb", "512", "--memory-mb", "1024"],
            vec!["mint", "--cpus", "2", "--cpus", "4"],
            vec!["mint", "--run-id", "one", "--run-id", "two"],
            vec!["mint", "--parent-lease", "one", "--parent-lease", "two"],
            vec![
                "mint",
                "--payload-manifest",
                "/one",
                "--payload-manifest",
                "/two",
            ],
            vec!["mint", "--payload-image", "/one", "--payload-image", "/two"],
            vec![
                "mint",
                "--guest-adapter",
                "debian-nocloud",
                "--guest-adapter",
                "mint-cinnamon",
            ],
            vec![
                "mint",
                "--guest-image-revision",
                "one",
                "--guest-image-revision",
                "two",
            ],
            vec!["mint", "--run-root", "/one", "--run-root", "/two"],
            vec!["mint", "--image-kind", "raw", "--image-kind", "qcow2"],
            vec![
                "mint",
                "--acceleration",
                "hardware",
                "--acceleration",
                "allow-tcg",
            ],
            vec!["mint", "--arch", "x86_64", "--arch", "aarch64"],
            vec!["mint", "--firmware", "bios", "--firmware", "uefi"],
            vec![
                "mint",
                "--environment-id",
                "linux/mint",
                "--environment-id",
                "linux/debian",
            ],
            vec!["mint", "--unknown"],
            vec!["mint", "--cpus=2"],
            vec!["mint", "--memory-mb"],
            vec!["mint", "--cpus", "--headless"],
            vec!["mint", "--run-id"],
            vec!["mint", "--parent-lease"],
            vec!["mint", "--guest-adapter"],
            vec!["mint", "--guest-image-revision"],
            vec!["mint", "--guest-image-revision", "../unsafe"],
            vec!["mint", "--payload-manifest"],
            vec!["mint", "--payload-manifest", "relative/manifest.json"],
            vec!["mint", "--payload-image"],
            vec!["mint", "--payload-image", "relative/payload.iso"],
            vec!["mint", "--run-root"],
            vec!["mint", "--image-kind"],
            vec!["mint", "--acceleration"],
            vec!["mint", "--environment-id"],
            vec!["mint", "--arch"],
            vec!["mint", "--firmware"],
            vec!["mint", "--arch", "amd64"],
            vec!["mint", "--firmware", "efi"],
            vec!["mint", "--image-kind", "vhdx"],
            vec!["mint", "--acceleration", "kvm"],
            vec!["mint", "--guest-adapter", "mint"],
        ];
        for values in invalid {
            assert!(parse(&values).is_err(), "values: {values:?}");
        }
    }

    #[test]
    fn rejects_missing_empty_and_multiple_targets() {
        let invalid = [vec![], vec![""], vec!["mint", "debian"]];
        for values in invalid {
            assert!(parse(&values).is_err(), "values: {values:?}");
        }
    }

    #[test]
    fn validates_run_ids_as_single_safe_segments() {
        let sixty_four = "a".repeat(MAX_RUN_ID_LEN);
        let sixty_five = "a".repeat(MAX_RUN_ID_LEN + 1);
        let valid = ["a", "A-Z_09", "lane-01", sixty_four.as_str()];
        for value in valid {
            assert!(parse(&["mint", "--run-id", value]).is_ok(), "{value:?}");
        }

        let invalid = [
            "",
            "lane/01",
            "lane.01",
            "lane 01",
            "'lane-01'",
            "lane\"01",
            "læne",
            "lane\n01",
            "lane\t01",
            "lane\u{1}",
            sixty_five.as_str(),
        ];
        for value in invalid {
            assert!(parse(&["mint", "--run-id", value]).is_err(), "{value:?}");
        }
    }

    #[test]
    fn validates_environment_ids_as_safe_logical_paths() {
        let valid = ["mint", "linux/mint", "linux/mint-22/cinnamon_6"];
        for value in valid {
            assert!(
                parse(&["/images/mint.qcow2", "--environment-id", value]).is_ok(),
                "{value:?}"
            );
        }

        let invalid = [
            "",
            "/mint",
            "mint/",
            "linux//mint",
            ".",
            "..",
            "linux/../mint",
            "linux/./mint",
            "linux\\mint",
            "linux mint",
            "linux/'mint'",
            "linux/mínt",
            "linux/mint\n",
            "linux/mint\t",
            "linux/mint\0",
        ];
        for value in invalid {
            assert!(
                parse(&["/images/mint.qcow2", "--environment-id", value]).is_err(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn accepts_unicode_quotes_and_spaces_in_an_opaque_target() {
        let target = "/images/Mínt “Lab”.qcow2";
        assert_eq!(parse(&[target]).unwrap().target, target);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_argv() {
        use std::os::unix::ffi::OsStringExt;

        let values = vec![OsString::from("mint"), OsString::from_vec(vec![0xff])];
        assert!(parse_launch_options(&values, "qol emu up <environment>").is_err());
    }
}
