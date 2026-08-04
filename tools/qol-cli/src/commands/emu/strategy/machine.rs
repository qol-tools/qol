use super::super::{AccelerationRequirement, BackendImageKind, BackendSpec, Firmware, GuestArch};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MachineBackend {
    Qemu,
    AppleVirtualization,
}

impl MachineBackend {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "qemu" => Some(Self::Qemu),
            "apple-virtualization" => Some(Self::AppleVirtualization),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Qemu => "qemu",
            Self::AppleVirtualization => "apple-virtualization",
        }
    }
}

pub(crate) trait MachineStrategy {
    fn backend(&self) -> MachineBackend;

    fn backend_spec(
        &self,
        image_kind: &str,
        arch: Option<&str>,
        firmware: Option<&str>,
        acceleration: Option<&str>,
    ) -> std::result::Result<BackendSpec, String>;
}

struct QemuMachineStrategy;

impl MachineStrategy for QemuMachineStrategy {
    fn backend(&self) -> MachineBackend {
        MachineBackend::Qemu
    }

    fn backend_spec(
        &self,
        image_kind: &str,
        arch: Option<&str>,
        firmware: Option<&str>,
        acceleration: Option<&str>,
    ) -> std::result::Result<BackendSpec, String> {
        let arch = arch.unwrap_or("x86_64");
        let arch = GuestArch::parse(arch)
            .ok_or_else(|| format!("unsupported image architecture `{arch}`"))?;
        let firmware = match firmware {
            Some(value) => {
                Firmware::parse(value).ok_or_else(|| format!("unsupported firmware `{value}`"))?
            }
            None => Firmware::for_arch(arch),
        };
        let image_kind = BackendImageKind::parse(image_kind)
            .ok_or_else(|| format!("unsupported image kind `{image_kind}`"))?;
        let acceleration = acceleration.unwrap_or("allow-tcg");
        let acceleration = AccelerationRequirement::parse(acceleration)
            .ok_or_else(|| format!("unsupported acceleration requirement `{acceleration}`"))?;
        Ok(BackendSpec::from_launch(
            arch,
            firmware,
            image_kind,
            acceleration,
        ))
    }
}

struct AppleVirtualizationMachineStrategy;

impl MachineStrategy for AppleVirtualizationMachineStrategy {
    fn backend(&self) -> MachineBackend {
        MachineBackend::AppleVirtualization
    }

    fn backend_spec(
        &self,
        _image_kind: &str,
        _arch: Option<&str>,
        _firmware: Option<&str>,
        _acceleration: Option<&str>,
    ) -> std::result::Result<BackendSpec, String> {
        Err(format!(
            "machine strategy `{}` is not available yet; Apple Virtualization.framework is the next backend",
            self.backend().as_str()
        ))
    }
}

static QEMU: QemuMachineStrategy = QemuMachineStrategy;
static APPLE_VIRTUALIZATION: AppleVirtualizationMachineStrategy =
    AppleVirtualizationMachineStrategy;

pub(crate) fn resolve_machine_strategy(
    backend: &str,
) -> std::result::Result<&'static dyn MachineStrategy, String> {
    let backend = MachineBackend::parse(backend).ok_or_else(|| {
        format!("unsupported machine backend `{backend}`; available: qemu, apple-virtualization")
    })?;
    match backend {
        MachineBackend::Qemu => Ok(&QEMU),
        MachineBackend::AppleVirtualization => Ok(&APPLE_VIRTUALIZATION),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_selects_current_and_future_machine_strategies() {
        assert_eq!(MachineBackend::parse("qemu"), Some(MachineBackend::Qemu));
        assert_eq!(
            MachineBackend::parse("apple-virtualization"),
            Some(MachineBackend::AppleVirtualization)
        );
        assert!(resolve_machine_strategy("qemu").is_ok());
        assert!(resolve_machine_strategy("apple-virtualization").is_ok());
    }

    #[test]
    fn apple_virtualization_strategy_is_explicitly_unavailable() {
        let error = resolve_machine_strategy("apple-virtualization")
            .unwrap()
            .backend_spec("macos-vm-bundle", Some("aarch64"), None, None)
            .unwrap_err();
        assert!(error.contains("not available yet"), "error: {error}");
        assert!(error.contains("Virtualization.framework"), "error: {error}");
    }
}
