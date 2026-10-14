const RESTARTS: [&str; 7] = [
    "glibc",
    "systemd",
    "dbus",
    "intel-microcode",
    "amd64-microcode",
    "linux-firmware",
    "nvidia-graphics-drivers",
];
const SOUND: [&str; 4] = ["bluez", "pipewire", "wireplumber", "pulseaudio"];
const NOT_KERNEL: [&str; 4] = [
    "linux-firmware",
    "linux-base",
    "linux-sound-base",
    "linux-atm",
];

pub(super) struct Naming {
    pub(super) label: String,
    pub(super) name: String,
    pub(super) restart: bool,
    pub(super) stops_sound: bool,
    pub(super) caution: Option<(String, String)>,
}

pub(super) fn base(package: &str) -> &str {
    package.split(':').next().unwrap_or(package)
}

pub(super) fn is_kernel(source: &str) -> bool {
    (source == "linux" || source.starts_with("linux-")) && !NOT_KERNEL.contains(&source)
}

fn is_nvidia(source: &str) -> bool {
    source.starts_with("nvidia-graphics-drivers")
}

pub(super) fn key(source: &str) -> String {
    if is_kernel(source) {
        "linux".into()
    } else {
        source.into()
    }
}

pub(super) fn short_version(version: &str) -> String {
    let version = version.split_once(':').map_or(version, |(_, rest)| rest);
    let version = version
        .rsplit_once("really")
        .map_or(version, |(_, rest)| rest);
    let upstream = version
        .rsplit_once('-')
        .map_or(version, |(upstream, _)| upstream);
    upstream
        .split(['+', '~'])
        .next()
        .unwrap_or(upstream)
        .to_string()
}

pub(super) fn two_parts(version: &str) -> String {
    short_version(version)
        .split('.')
        .take(2)
        .collect::<Vec<_>>()
        .join(".")
}

pub(super) fn kernel_abi(package: &str) -> Option<String> {
    let rest = base(package).strip_prefix("linux-image-")?;
    let mut parts = rest.splitn(3, '-');
    let version = parts.next()?;
    let build = parts.next()?;
    (version.split('.').all(|part| part.parse::<u32>().is_ok()) && build.parse::<u32>().is_ok())
        .then(|| format!("{version}-{build}"))
}

pub(super) fn naming(source: &str, version: &str, packages: &[String]) -> Naming {
    let short = short_version(version);
    let (label, name) = if is_kernel(source) {
        let label = packages
            .iter()
            .find_map(|package| kernel_abi(package))
            .map_or_else(
                || "Linux kernel".to_string(),
                |abi| format!("Linux kernel {abi}"),
            );
        (label.clone(), label)
    } else if is_nvidia(source) {
        let label = format!("NVIDIA driver {}", two_parts(version));
        (label.clone(), label)
    } else if let Some(module) = source
        .strip_prefix("python3-")
        .or_else(|| source.strip_prefix("python-"))
    {
        (format!("{module} {short}"), module.to_string())
    } else {
        let pretty = match source {
            "apparmor" => "AppArmor",
            "bluez" => "Bluetooth",
            "firefox" => "Firefox",
            "google-chrome-stable" => "Chrome",
            "thunderbird" => "Thunderbird",
            "linux-firmware" => "Linux firmware",
            other => other,
        };
        let label = format!("{pretty} {short}");
        (label.clone(), label)
    };
    Naming {
        label,
        name,
        restart: is_kernel(source)
            || RESTARTS
                .iter()
                .any(|restart| source == *restart || source.starts_with(&format!("{restart}-"))),
        stops_sound: SOUND.contains(&source),
        caution: is_nvidia(source).then(|| {
            (
                "Games opened before the restart may fail".to_string(),
                "The new NVIDIA driver loads at the restart; open apps keep working".to_string(),
            )
        }),
    }
}

pub(super) fn family(package: &str) -> String {
    let package = base(package);
    let family = if package.starts_with("bluez")
        || package == "bluetooth"
        || package.starts_with("libbluetooth")
    {
        "Bluetooth"
    } else if package.starts_with("linux-") && !NOT_KERNEL.contains(&package) {
        "kernel"
    } else if package.starts_with("firefox") {
        "Firefox"
    } else if package.starts_with("google-chrome") {
        "Chrome"
    } else if package.starts_with("thunderbird") {
        "Thunderbird"
    } else if package.starts_with("nvidia-") || package.starts_with("libnvidia-") {
        "NVIDIA"
    } else {
        package
    };
    family.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(source: &str, version: &str, packages: &[&str]) -> (String, String, bool) {
        let packages: Vec<String> = packages.iter().map(|package| package.to_string()).collect();
        let naming = naming(source, version, &packages);
        (naming.label, naming.name, naming.restart)
    }

    #[test]
    fn versions_lose_epochs_revisions_and_backport_tails() {
        assert_eq!(short_version("2.0.7-1ubuntu0.8"), "2.0.7");
        assert_eq!(short_version("4.0.1really4.0.1-0ubuntu0.24.04.9"), "4.0.1");
        assert_eq!(short_version("2:1.8.7-1ubuntu0.1"), "1.8.7");
        assert_eq!(short_version("2.4.2-1~ubuntu.24.04~noble"), "2.4.2");
        assert_eq!(short_version("158.0+linuxmint1"), "158.0");
        assert_eq!(short_version("1.0.10"), "1.0.10");
        assert_eq!(two_parts("580.178.04-0ubuntu0.24.04.1"), "580.178");
    }

    #[test]
    fn updates_read_like_the_design() {
        assert_eq!(
            names(
                "nvidia-graphics-drivers-580",
                "580.178.04-0ubuntu0.24.04.1",
                &["nvidia-driver-580"]
            ),
            (
                "NVIDIA driver 580.178".into(),
                "NVIDIA driver 580.178".into(),
                true
            )
        );
        assert_eq!(
            names("python-urllib3", "2.0.7-1ubuntu0.8", &["python3-urllib3"]),
            ("urllib3 2.0.7".into(), "urllib3".into(), false)
        );
        assert_eq!(
            names(
                "apparmor",
                "4.0.1really4.0.1-0ubuntu0.24.04.9",
                &["apparmor"]
            ),
            ("AppArmor 4.0.1".into(), "AppArmor 4.0.1".into(), false)
        );
        assert_eq!(
            names(
                "linux-meta-lowlatency",
                "6.8.0-148.148.1",
                &["linux-lowlatency", "linux-image-6.8.0-148-lowlatency"]
            ),
            (
                "Linux kernel 6.8.0-148".into(),
                "Linux kernel 6.8.0-148".into(),
                true
            )
        );
        assert_eq!(
            names(
                "containerd.io",
                "2.4.2-1~ubuntu.24.04~noble",
                &["containerd.io"]
            )
            .0,
            "containerd.io 2.4.2"
        );
        assert!(names("systemd", "255.4-1ubuntu8.9", &["systemd"]).2);
        assert!(!names("linux-base", "4.5ubuntu9", &["linux-base"]).2);
    }

    #[test]
    fn kernels_group_together_and_sound_is_known() {
        assert_eq!(key("linux-signed-lowlatency"), "linux");
        assert_eq!(key("linux-firmware"), "linux-firmware");
        assert!(naming("bluez", "5.72-0ubuntu5.8", &[]).stops_sound);
        assert!(naming(
            "nvidia-graphics-drivers-580",
            "580.178.04-0ubuntu0.24.04.1",
            &[]
        )
        .caution
        .is_some());
        assert_eq!(
            kernel_abi("linux-image-6.8.0-147-lowlatency:amd64").as_deref(),
            Some("6.8.0-147")
        );
        assert_eq!(kernel_abi("linux-image-lowlatency"), None);
    }

    #[test]
    fn history_families_match_the_last_run_line() {
        let families: Vec<String> = [
            "bluez:amd64",
            "libbluetooth3:amd64",
            "bluetooth:amd64",
            "linux-image-lowlatency:amd64",
            "firefox-locale-en:amd64",
            "google-chrome-stable:amd64",
            "python3-jwt:all",
        ]
        .iter()
        .map(|package| family(package))
        .collect();
        assert_eq!(
            families,
            [
                "Bluetooth",
                "Bluetooth",
                "Bluetooth",
                "kernel",
                "Firefox",
                "Chrome",
                "python3-jwt"
            ]
        );
    }
}
