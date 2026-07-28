use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::features::plugin_store) struct PlatformTarget {
    pub(in crate::features::plugin_store) os: SupportedOs,
    pub(in crate::features::plugin_store) arch: SupportedArch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::features::plugin_store) enum SupportedOs {
    Linux,
    Macos,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::features::plugin_store) enum SupportedArch {
    X86_64,
    Aarch64,
}

impl PlatformTarget {
    pub(in crate::features::plugin_store) fn current() -> Result<Self> {
        super::super::platform::release_target()
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
    pub(in crate::features::plugin_store) fn from_token(token: &str) -> Result<Self> {
        match token {
            "linux" => Ok(Self::Linux),
            "macos" => Ok(Self::Macos),
            "windows" => Ok(Self::Windows),
            other => anyhow::bail!("unsupported OS for release asset resolution: {}", other),
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
    pub(in crate::features::plugin_store) fn from_token(token: &str) -> Result<Self> {
        match token {
            "x86_64" => Ok(Self::X86_64),
            "aarch64" => Ok(Self::Aarch64),
            other => anyhow::bail!("unsupported CPU architecture for release assets: {}", other),
        }
    }

    fn token(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }
}

pub(in crate::features::plugin_store) fn resolve_asset_pattern(
    pattern: &str,
    target: PlatformTarget,
) -> String {
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
