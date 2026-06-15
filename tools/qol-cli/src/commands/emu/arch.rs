#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestArch {
    X86_64,
    Aarch64,
}

impl GuestArch {
    pub(crate) const ALL: [GuestArch; 2] = [GuestArch::X86_64, GuestArch::Aarch64];

    pub(crate) fn parse(value: &str) -> Option<GuestArch> {
        match value {
            "x86_64" => Some(GuestArch::X86_64),
            "aarch64" => Some(GuestArch::Aarch64),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            GuestArch::X86_64 => "x86_64",
            GuestArch::Aarch64 => "aarch64",
        }
    }

    pub(crate) fn toggled(self) -> GuestArch {
        match self {
            GuestArch::X86_64 => GuestArch::Aarch64,
            GuestArch::Aarch64 => GuestArch::X86_64,
        }
    }

    pub(crate) fn qemu_system_binary(self) -> &'static str {
        match self {
            GuestArch::X86_64 => "qemu-system-x86_64",
            GuestArch::Aarch64 => "qemu-system-aarch64",
        }
    }

    pub(crate) fn machine_type(self) -> &'static str {
        match self {
            GuestArch::X86_64 => "q35",
            GuestArch::Aarch64 => "virt",
        }
    }

    pub(crate) fn firmware_file(self, firmware: Firmware) -> Vec<&'static str> {
        match (self, firmware) {
            (GuestArch::X86_64, Firmware::Bios) => vec![],
            (GuestArch::X86_64, Firmware::Uefi) => {
                vec!["edk2-x86_64-code.fd", "OVMF_CODE.fd", "OVMF_CODE_4M.fd"]
            }
            (GuestArch::Aarch64, Firmware::Bios | Firmware::Uefi) => {
                vec!["edk2-aarch64-code.fd"]
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Firmware {
    Bios,
    Uefi,
}

impl Firmware {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Firmware::Bios => "bios",
            Firmware::Uefi => "uefi",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Firmware> {
        match value {
            "bios" => Some(Firmware::Bios),
            "uefi" => Some(Firmware::Uefi),
            _ => None,
        }
    }

    pub(crate) fn for_arch(arch: GuestArch) -> Firmware {
        match arch {
            GuestArch::X86_64 => Firmware::Bios,
            GuestArch::Aarch64 => Firmware::Uefi,
        }
    }
}

pub(crate) fn infer_arch_from_filename(name: &str) -> Option<GuestArch> {
    let lower = name.to_ascii_lowercase();
    let contains = |needle: &str| lower.contains(needle);
    if contains("arm64") || contains("aarch64") {
        return Some(GuestArch::Aarch64);
    }
    if contains("amd64")
        || contains("x86_64")
        || contains("x64")
        || contains("i386")
        || contains("i686")
    {
        return Some(GuestArch::X86_64);
    }
    None
}

pub(crate) fn is_windows_image_hint(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("win") || lower.contains("windows") || lower.ends_with(".vhdx")
}

pub(crate) fn infer_firmware(arch: GuestArch, name: &str) -> Firmware {
    match arch {
        GuestArch::Aarch64 => Firmware::Uefi,
        GuestArch::X86_64 => {
            if is_windows_image_hint(name) {
                Firmware::Uefi
            } else {
                Firmware::Bios
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_supported_arches() {
        let cases = [
            ("x86_64", Some(GuestArch::X86_64)),
            ("aarch64", Some(GuestArch::Aarch64)),
            ("arm64", None),
            ("", None),
        ];
        for (input, expected) in cases {
            assert_eq!(GuestArch::parse(input), expected, "input: {input}");
        }
    }

    #[test]
    fn toggled_swaps_between_arches() {
        let cases = [
            (GuestArch::X86_64, GuestArch::Aarch64),
            (GuestArch::Aarch64, GuestArch::X86_64),
        ];
        for (arch, expected) in cases {
            assert_eq!(arch.toggled(), expected, "arch: {arch:?}");
        }
    }

    #[test]
    fn qemu_system_binary_is_arch_suffixed() {
        let cases = [
            (GuestArch::X86_64, "qemu-system-x86_64"),
            (GuestArch::Aarch64, "qemu-system-aarch64"),
        ];
        for (arch, expected) in cases {
            assert_eq!(arch.qemu_system_binary(), expected, "arch: {arch:?}");
        }
    }

    #[test]
    fn firmware_file_selection_by_arch_and_mode() {
        let cases: [(GuestArch, Firmware, Vec<&str>); 4] = [
            (GuestArch::X86_64, Firmware::Bios, vec![]),
            (
                GuestArch::X86_64,
                Firmware::Uefi,
                vec!["edk2-x86_64-code.fd", "OVMF_CODE.fd", "OVMF_CODE_4M.fd"],
            ),
            (
                GuestArch::Aarch64,
                Firmware::Bios,
                vec!["edk2-aarch64-code.fd"],
            ),
            (
                GuestArch::Aarch64,
                Firmware::Uefi,
                vec!["edk2-aarch64-code.fd"],
            ),
        ];
        for (arch, firmware, expected) in cases {
            assert_eq!(
                GuestArch::firmware_file(arch, firmware),
                expected,
                "arch: {arch:?}, firmware: {firmware:?}"
            );
        }
    }

    #[test]
    fn firmware_as_str_and_parse_round_trip() {
        let cases = [(Firmware::Bios, "bios"), (Firmware::Uefi, "uefi")];
        for (firmware, expected) in cases {
            assert_eq!(firmware.as_str(), expected, "firmware: {firmware:?}");
            assert_eq!(
                Firmware::parse(expected),
                Some(firmware),
                "input: {expected}"
            );
        }
        assert_eq!(Firmware::parse("legacy"), None);
        assert_eq!(Firmware::parse(""), None);
    }

    #[test]
    fn firmware_for_arch_defaults_uefi_on_arm_bios_on_x86() {
        let cases = [
            (GuestArch::X86_64, Firmware::Bios),
            (GuestArch::Aarch64, Firmware::Uefi),
        ];
        for (arch, expected) in cases {
            assert_eq!(Firmware::for_arch(arch), expected, "arch: {arch:?}");
        }
    }

    #[test]
    fn infers_arch_from_filename_tokens() {
        let cases = [
            ("ubuntu-arm64.qcow2", Some(GuestArch::Aarch64)),
            ("debian-aarch64.img", Some(GuestArch::Aarch64)),
            ("win11-amd64.vhdx", Some(GuestArch::X86_64)),
            ("fedora-x86_64.raw", Some(GuestArch::X86_64)),
            ("disk-x64.qcow2", Some(GuestArch::X86_64)),
            ("legacy-i386.img", Some(GuestArch::X86_64)),
            ("old-i686.img", Some(GuestArch::X86_64)),
            ("mystery.qcow2", None),
        ];
        for (name, expected) in cases {
            assert_eq!(infer_arch_from_filename(name), expected, "name: {name}");
        }
    }

    #[test]
    fn detects_windows_image_hint() {
        let cases = [
            ("win11.qcow2", true),
            ("windows-server.img", true),
            ("disk.vhdx", true),
            ("ubuntu.qcow2", false),
        ];
        for (name, expected) in cases {
            assert_eq!(is_windows_image_hint(name), expected, "name: {name}");
        }
    }

    #[test]
    fn infers_firmware_per_arch_and_windows_hint() {
        let cases = [
            (GuestArch::Aarch64, "ubuntu-arm64.qcow2", Firmware::Uefi),
            (GuestArch::X86_64, "ubuntu.qcow2", Firmware::Bios),
            (GuestArch::X86_64, "win11.vhdx", Firmware::Uefi),
            (GuestArch::X86_64, "windows-server.qcow2", Firmware::Uefi),
        ];
        for (arch, name, expected) in cases {
            assert_eq!(infer_firmware(arch, name), expected, "name: {name}");
        }
    }
}
