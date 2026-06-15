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

    pub(crate) fn firmware_file(self) -> Option<&'static str> {
        match self {
            GuestArch::X86_64 => None,
            GuestArch::Aarch64 => Some("edk2-aarch64-code.fd"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Firmware {
    Bios,
    Uefi,
}

#[allow(dead_code)]
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
}
