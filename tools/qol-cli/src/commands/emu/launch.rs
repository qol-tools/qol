use anyhow::{anyhow, bail, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

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
    #[default]
    Host,
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
    pub(crate) memory_mb: u32,
    pub(crate) cpus: u16,
    pub(crate) run_id: Option<String>,
    pub(crate) run_root: Option<PathBuf>,
    pub(crate) image_kind: Option<BackendImageKind>,
    pub(crate) acceleration: AccelerationRequirement,
    pub(crate) arch: Option<GuestArch>,
    pub(crate) firmware: Option<Firmware>,
}

impl LaunchOptions {
    pub(crate) fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            environment_id: None,
            display: DisplayMode::Host,
            memory_mb: DEFAULT_MEMORY_MB,
            cpus: DEFAULT_CPUS,
            run_id: None,
            run_root: None,
            image_kind: None,
            acceleration: AccelerationRequirement::AllowTcg,
            arch: None,
            firmware: None,
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
    pub(crate) run_root: Option<&'a Path>,
    pub(crate) image_kind: Option<&'a str>,
    pub(crate) display: DisplayMode,
    pub(crate) resources: ResourceProfile,
    pub(crate) acceleration: Option<&'a str>,
    pub(crate) arch: Option<&'a str>,
    pub(crate) firmware: Option<&'a str>,
}

pub(crate) fn child_args(launch: ChildLaunch<'_>) -> Result<Vec<OsString>> {
    validate_run_id(launch.run_id)?;
    validate_environment_id(launch.environment_id)?;
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
    if launch.display == DisplayMode::None {
        args.push(OsString::from("--headless"));
    }
    args.extend([
        OsString::from("--memory-mb"),
        OsString::from(launch.resources.memory_mb.to_string()),
        OsString::from("--cpus"),
        OsString::from(launch.resources.cpus.to_string()),
        OsString::from("--run-id"),
        OsString::from(launch.run_id),
        OsString::from("--environment-id"),
        OsString::from(launch.environment_id),
    ]);
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
    Ok(args)
}

pub(crate) fn parse_launch_options(args: &[OsString], usage: &str) -> Result<LaunchOptions> {
    let mut target = None;
    let mut environment_id = None;
    let mut display = None;
    let mut memory_mb = None;
    let mut cpus = None;
    let mut run_id = None;
    let mut run_root = None;
    let mut image_kind = None;
    let mut acceleration = None;
    let mut arch = None;
    let mut firmware = None;
    let mut index = 0;

    while index < args.len() {
        let argument = utf8_arg(&args[index])?;
        match argument {
            "--headless" => {
                reject_duplicate(display.is_some(), "--headless")?;
                display = Some(DisplayMode::None);
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
    options.memory_mb = memory_mb.unwrap_or(DEFAULT_MEMORY_MB);
    options.cpus = cpus.unwrap_or(DEFAULT_CPUS);
    options.run_id = run_id;
    options.run_root = run_root;
    options.image_kind = image_kind;
    options.acceleration = acceleration.unwrap_or(AccelerationRequirement::AllowTcg);
    options.arch = arch;
    options.firmware = firmware;
    Ok(options)
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

    fn child_launch<'a>(operation: ChildOperation<'a>) -> ChildLaunch<'a> {
        ChildLaunch {
            operation,
            target: Path::new("/images/debian.qcow2"),
            environment_id: "linux/debian",
            run_id: "debian-lane-1",
            run_root: Some(Path::new("/runs/cases")),
            image_kind: Some("qcow2"),
            display: DisplayMode::None,
            resources: ResourceProfile {
                memory_mb: 768,
                cpus: 2,
            },
            acceleration: Some("hardware"),
            arch: Some("x86_64"),
            firmware: Some("bios"),
        }
    }

    #[test]
    fn child_launch_args_are_shared_by_env_and_flow() {
        let up = child_args(child_launch(ChildOperation::Up)).unwrap();
        assert_eq!(
            up,
            argv(&[
                "emu",
                "up",
                "/images/debian.qcow2",
                "--headless",
                "--memory-mb",
                "768",
                "--cpus",
                "2",
                "--run-id",
                "debian-lane-1",
                "--environment-id",
                "linux/debian",
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

        let run = child_args(child_launch(ChildOperation::Run("leaves-no-trace"))).unwrap();
        assert_eq!(&run[..3], &argv(&["emu", "run", "leaves-no-trace"]));
        assert_eq!(&run[3..], &up[2..]);
    }

    #[test]
    fn child_launch_reuses_identity_and_manifest_validation() {
        let mut cases = [
            child_launch(ChildOperation::Up),
            child_launch(ChildOperation::Up),
            child_launch(ChildOperation::Run("")),
            child_launch(ChildOperation::Up),
        ];
        cases[0].run_id = "bad/run";
        cases[1].environment_id = "../bad";
        cases[2].arch = None;
        cases[3].firmware = Some("efi");
        for launch in cases {
            assert!(child_args(launch).is_err());
        }

        let mut invalid_kind = child_launch(ChildOperation::Up);
        invalid_kind.image_kind = Some("vhdx");
        assert!(child_args(invalid_kind).is_err());

        let mut invalid_acceleration = child_launch(ChildOperation::Up);
        invalid_acceleration.acceleration = Some("kvm");
        assert!(child_args(invalid_acceleration).is_err());
    }

    #[test]
    fn parses_defaults_and_options_in_any_position() {
        let default = parse(&["mint"]).unwrap();
        assert_eq!(default, LaunchOptions::new("mint"));

        let cases = [
            vec![
                "mint",
                "--headless",
                "--memory-mb",
                "8192",
                "--cpus",
                "8",
                "--run-id",
                "lane_01",
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
            ],
        ];
        for values in cases {
            let parsed = parse(&values).unwrap();
            assert_eq!(parsed.target, "mint", "values: {values:?}");
            assert_eq!(parsed.environment_id.as_deref(), Some("linux/mint"));
            assert_eq!(parsed.display, DisplayMode::None);
            assert_eq!(parsed.memory_mb, 8192);
            assert_eq!(parsed.cpus, 8);
            assert_eq!(parsed.run_id.as_deref(), Some("lane_01"));
            assert_eq!(parsed.run_root.as_deref(), Some(Path::new("/runs/mint")));
            assert_eq!(parsed.image_kind, Some(BackendImageKind::Qcow2));
            assert_eq!(parsed.acceleration, AccelerationRequirement::Hardware);
            assert_eq!(parsed.arch, Some(GuestArch::X86_64));
            assert_eq!(parsed.firmware, Some(Firmware::Uefi));
        }
    }

    #[test]
    fn produces_qemu_values_for_host_and_headless_modes() {
        let host = LaunchOptions::new("mint");
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
            vec!["mint", "--memory-mb", "512", "--memory-mb", "1024"],
            vec!["mint", "--cpus", "2", "--cpus", "4"],
            vec!["mint", "--run-id", "one", "--run-id", "two"],
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
