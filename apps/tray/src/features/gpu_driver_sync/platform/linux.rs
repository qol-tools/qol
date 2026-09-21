use super::Observation;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) const MODINFO_CANDIDATES: [&str; 2] = ["/usr/sbin/modinfo", "/sbin/modinfo"];
#[cfg(not(test))]
const MODINFO_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
#[cfg(test)]
const MODINFO_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(300);
const MODINFO_OUTPUT_LIMIT: usize = 4096;

pub(crate) fn watch_supported() -> bool {
    true
}

pub(crate) fn observe() -> Observation {
    observe_from(&proc_version_path())
}

fn observe_from(path: &Path) -> Observation {
    match std::fs::read_to_string(path) {
        Ok(text) => match parse_proc_version(&text) {
            Some(loaded) => classify(loaded, on_disk_version()),
            None => Observation::LoadedUnavailable,
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Observation::NotLoaded,
        Err(_) => Observation::LoadedUnavailable,
    }
}

fn proc_version_path() -> PathBuf {
    #[cfg(feature = "dev")]
    if let Some(path) = std::env::var_os("QOL_NVIDIA_PROC_VERSION") {
        return PathBuf::from(path);
    }
    PathBuf::from("/proc/driver/nvidia/version")
}

fn on_disk_version() -> Option<String> {
    MODINFO_CANDIDATES
        .iter()
        .find_map(|binary| bounded_modinfo_version(Path::new(binary)))
}

pub(crate) fn bounded_modinfo_version(binary: &Path) -> Option<String> {
    let mut command = Command::new(binary);
    command.args(["-F", "version", "nvidia"]);
    let output = qol_process::run_guarded_with_output_timeout(
        command,
        modinfo_guardian_command()?,
        MODINFO_PROBE_TIMEOUT,
        MODINFO_OUTPUT_LIMIT,
    )
    .ok()?;
    let qol_process::BoundedCommandOutput::Completed(output) = output else {
        return None;
    };
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(output.stdout.as_bytes())
        .trim()
        .to_string();
    is_version_token(&version).then_some(version)
}

fn modinfo_guardian_command() -> Option<Command> {
    let current = std::env::current_exe().ok()?;
    #[cfg(not(test))]
    {
        Some(qol_process::process_tree_guardian_command(&current))
    }
    #[cfg(test)]
    {
        let mut command = Command::new(current);
        command.args([
            "--exact",
            "features::gpu_driver_sync::platform::linux::tests::guarded_modinfo_helper",
            "--nocapture",
        ]);
        Some(command)
    }
}

fn classify(loaded: String, on_disk: Option<String>) -> Observation {
    match on_disk {
        Some(on_disk) if on_disk == loaded => Observation::Matched { loaded },
        Some(on_disk) => Observation::Mismatch { loaded, on_disk },
        None => Observation::OnDiskUnavailable { loaded },
    }
}

fn parse_proc_version(text: &str) -> Option<String> {
    text.lines()
        .find(|line| line.starts_with("NVRM version:"))?
        .split_whitespace()
        .find(|token| is_version_token(token))
        .map(str::to_string)
}

fn is_version_token(token: &str) -> bool {
    let mut parts = token.split('.').peekable();
    let Some(first) = parts.next() else {
        return false;
    };
    if first.is_empty() || !first.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    if parts.peek().is_none() {
        return false;
    }
    parts.all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_modinfo_helper() {
        if std::env::var_os("QOL_PROCESS_GUARDIAN_PROTOCOL").is_none() {
            return;
        }
        qol_process::run_process_tree_guardian_entry().unwrap();
    }

    #[test]
    fn modinfo_resolution_never_consults_the_caller_path() {
        assert!(
            !MODINFO_CANDIDATES
                .iter()
                .any(|candidate| !candidate.starts_with('/')),
            "every modinfo candidate must be an absolute system path"
        );
    }

    #[test]
    fn a_hanging_modinfo_probe_is_bounded_and_leaves_no_child() {
        let dir = std::env::temp_dir().join(format!("qol-modinfo-hang-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("modinfo");
        let pidfile = dir.join("pid");
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho $$ > \"{}\"\nsleep 30\n", pidfile.display()),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        let started = std::time::Instant::now();
        let version = bounded_modinfo_version(&script);
        assert_eq!(
            version, None,
            "a hanging modinfo probe must be aborted, not answered"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "the hanging probe must be terminated within the short test bound"
        );
        let pid: u32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(
            !qol_process::is_pid_alive(pid),
            "the contained probe tree must be reaped before the guarded call returns, leaving no orphan"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_working_modinfo_probe_returns_the_version() {
        let dir = std::env::temp_dir().join(format!("qol-modinfo-ok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("modinfo");
        std::fs::write(
            &script,
            "#!/bin/sh\nif [ \"$1\" = -F ] && [ \"$2\" = version ]; then echo 580.159.02; exit 0; fi\nexit 1\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        assert_eq!(
            bounded_modinfo_version(&script).as_deref(),
            Some("580.159.02")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn proc_version_parsing_extracts_the_module_version() {
        let cases: [(&str, Option<&str>); 8] = [
            (
                "NVRM version: NVIDIA UNIX x86_64 Kernel Module  580.159.02  Wed May 14 21:38:31 UTC 2025\nGCC version:  gcc version 13.3.0",
                Some("580.159.02"),
            ),
            (
                "NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  580.65.06  Release Build",
                Some("580.65.06"),
            ),
            ("NVRM version: 560.35.02", Some("560.35.02")),
            ("GCC version:  gcc version 13.3.0", None),
            ("NVRM version: NVIDIA UNIX Kernel Module", None),
            ("NVRM version: NVIDIA 580", None),
            ("NVRM version:", None),
            ("", None),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_proc_version(input).as_deref(), expected, "{input:?}");
        }
    }

    #[test]
    fn version_token_accepts_only_dotted_numerics() {
        let cases: [(&str, bool); 12] = [
            ("580.159.02", true),
            ("580.65", true),
            ("5.8.0", true),
            ("580", false),
            ("x86_64", false),
            ("580.159.", false),
            (".580", false),
            ("580..02", false),
            ("Module", false),
            ("580.abc", false),
            ("560.35.03-0ubuntu1", false),
            ("", false),
        ];
        for (input, expected) in cases {
            assert_eq!(is_version_token(input), expected, "{input:?}");
        }
    }

    #[test]
    fn observe_maps_a_missing_proc_file_to_not_loaded() {
        let path = std::env::temp_dir().join(format!("qol-gpu-absent-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert_eq!(observe_from(&path), Observation::NotLoaded);
    }

    #[test]
    fn observe_maps_non_regular_read_sources_to_loaded_unavailable() {
        let path = std::env::temp_dir().join(format!("qol-gpu-dir-{}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create fixture directory");
        let observed = observe_from(&path);
        std::fs::remove_dir_all(&path).ok();
        assert_eq!(
            observed,
            Observation::LoadedUnavailable,
            "a directory read must fail deterministically instead of classifying as NotLoaded, even under root"
        );
    }

    #[test]
    fn observe_maps_unparseable_proc_content_to_loaded_unavailable() {
        let path = std::env::temp_dir().join(format!("qol-gpu-garbage-{}", std::process::id()));
        std::fs::write(&path, "not an NVRM version line\n").expect("write fixture");
        assert_eq!(observe_from(&path), Observation::LoadedUnavailable);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn classify_maps_on_disk_outcomes() {
        let cases: [(Option<&str>, Observation); 3] = [
            (
                Some("580.159.02"),
                Observation::Matched {
                    loaded: "580.159.02".to_string(),
                },
            ),
            (
                Some("580.173.00"),
                Observation::Mismatch {
                    loaded: "580.159.02".to_string(),
                    on_disk: "580.173.00".to_string(),
                },
            ),
            (
                None,
                Observation::OnDiskUnavailable {
                    loaded: "580.159.02".to_string(),
                },
            ),
        ];
        for (on_disk, expected) in cases {
            assert_eq!(
                classify("580.159.02".to_string(), on_disk.map(str::to_string)),
                expected,
                "{on_disk:?}"
            );
        }
    }
}
