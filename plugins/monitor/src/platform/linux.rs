use super::PlatformSupport;
use crate::monitor::backends::i2c_ddc::{I2cDdcBackend, LinuxI2cTransport};

pub(crate) fn current_support() -> PlatformSupport {
    PlatformSupport {
        name: "linux",
        supported: true,
    }
}

pub(crate) fn control() -> I2cDdcBackend<LinuxI2cTransport> {
    I2cDdcBackend::new(LinuxI2cTransport)
}
