use super::{HostArch, HostOs, Platform, PluginStorePlatformOps};

pub(super) trait PluginStoreDevPlatformOps {
    fn bind_public_runtime_socket(&self) -> bool;
}

pub(in crate::features::plugin_store) fn fixture_bundle_name() -> String {
    fixture_bundle_name_for(Platform.host_os(), HostArch::current())
}

fn fixture_bundle_name_for(host_os: HostOs, host_arch: HostArch) -> String {
    format!(
        "qol-tray-{}-{}",
        fixture_os_token(host_os),
        fixture_arch_token(host_arch)
    )
}

fn fixture_os_token(host_os: HostOs) -> &'static str {
    match host_os {
        HostOs::Linux => "linux",
        HostOs::Macos => "macos",
        HostOs::Windows => "windows",
        HostOs::Unsupported(_) => "linux",
    }
}

fn fixture_arch_token(host_arch: HostArch) -> &'static str {
    match host_arch {
        HostArch::Aarch64 => "aarch64",
        HostArch::X86_64 | HostArch::Unsupported(_) => "x86_64",
    }
}

pub(in crate::features::plugin_store) fn bind_public_runtime_socket() -> bool {
    Platform.bind_public_runtime_socket()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_hosts_keep_legacy_fixture_fallback() {
        assert_eq!(
            fixture_bundle_name_for(
                HostOs::Unsupported("dragonfly"),
                HostArch::Unsupported("riscv64")
            ),
            "qol-tray-linux-x86_64"
        );
    }
}
