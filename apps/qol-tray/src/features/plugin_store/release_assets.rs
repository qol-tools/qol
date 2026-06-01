use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PlatformTarget {
    os: SupportedOs,
    arch: SupportedArch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportedOs {
    Linux,
    Macos,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportedArch {
    X86_64,
    Aarch64,
}

impl PlatformTarget {
    pub(super) fn current() -> Result<Self> {
        Ok(Self {
            os: SupportedOs::current()?,
            arch: SupportedArch::current()?,
        })
    }

    fn os_token(self) -> &'static str {
        self.os.token()
    }

    fn arch_token(self) -> &'static str {
        self.arch.token()
    }

    fn executable_extension(self) -> &'static str {
        self.os.executable_extension()
    }
}

impl SupportedOs {
    fn current() -> Result<Self> {
        let os = std::env::consts::OS;
        if os == Self::Linux.token() {
            Ok(Self::Linux)
        } else if os == Self::Macos.token() {
            Ok(Self::Macos)
        } else if os == Self::Windows.token() {
            Ok(Self::Windows)
        } else {
            anyhow::bail!("unsupported OS for release asset resolution: {}", os)
        }
    }

    fn token(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }

    fn executable_extension(self) -> &'static str {
        match self {
            Self::Windows => ".exe",
            Self::Linux | Self::Macos => "",
        }
    }
}

impl SupportedArch {
    fn current() -> Result<Self> {
        let arch = std::env::consts::ARCH;
        if arch == Self::X86_64.token() {
            Ok(Self::X86_64)
        } else if arch == Self::Aarch64.token() {
            Ok(Self::Aarch64)
        } else {
            anyhow::bail!("unsupported CPU architecture for release assets: {}", arch)
        }
    }

    fn token(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }
}

pub(super) fn resolve_asset_pattern(pattern: &str, target: PlatformTarget) -> String {
    pattern
        .replace("{os}", target.os_token())
        .replace("{arch}", target.arch_token())
        + target.executable_extension()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_asset_pattern_linux_x86_64() {
        let target = PlatformTarget {
            os: SupportedOs::Linux,
            arch: SupportedArch::X86_64,
        };
        assert_eq!(
            resolve_asset_pattern("alt-tab-{os}-{arch}", target),
            "alt-tab-linux-x86_64"
        );
    }

    #[test]
    fn resolve_asset_pattern_windows_appends_exe() {
        let target = PlatformTarget {
            os: SupportedOs::Windows,
            arch: SupportedArch::Aarch64,
        };
        assert_eq!(
            resolve_asset_pattern("launcher-{os}-{arch}", target),
            "launcher-windows-aarch64.exe"
        );
    }
}
