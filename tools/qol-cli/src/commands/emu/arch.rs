#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestArch {
    X86_64,
    Aarch64,
}

impl GuestArch {
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
}
