use crate::doctor::device_permission::I2cProbe;
use std::io;

pub struct UnsupportedI2cProbe;

impl I2cProbe for UnsupportedI2cProbe {
    fn probe(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "device permission checks require Linux",
        ))
    }
}
