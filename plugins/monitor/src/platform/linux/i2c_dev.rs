use std::io;
use std::path::Path;

use crate::monitor::backends::i2c_ddc::{I2cBus, I2cTransport};
use crate::monitor::I2cError;

const I2C_SLAVE_REQUEST: libc::c_ulong = 0x0703;
const DDC_CI_SLAVE_ADDRESS: u8 = 0x37;

pub struct LinuxI2cTransport;

pub struct I2cFileBus {
    file: std::fs::File,
    node: String,
}

impl I2cTransport for LinuxI2cTransport {
    type Bus = I2cFileBus;

    fn open(&self, dev: &Path) -> Result<Self::Bus, I2cError> {
        use std::os::fd::AsRawFd;

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(dev)
            .map_err(|error| tier(&dev.display().to_string(), error))?;
        let result = unsafe {
            libc::ioctl(
                file.as_raw_fd(),
                I2C_SLAVE_REQUEST,
                DDC_CI_SLAVE_ADDRESS as libc::c_ulong,
            )
        };
        if result < 0 {
            return Err(tier(&dev.display().to_string(), io::Error::last_os_error()));
        }
        Ok(I2cFileBus {
            file,
            node: dev.display().to_string(),
        })
    }
}

impl I2cBus for I2cFileBus {
    fn write(&mut self, frame: &[u8]) -> Result<(), I2cError> {
        use std::io::Write;

        let written = self
            .file
            .write(frame)
            .map_err(|error| tier(&self.node, error))?;
        if written != frame.len() {
            return Err(I2cError::Protocol {
                detail: format!(
                    "short DDC/CI write on {}: wrote {written} of {} bytes",
                    self.node,
                    frame.len()
                ),
            });
        }
        Ok(())
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, I2cError> {
        use std::io::Read;

        self.file
            .read(buffer)
            .map_err(|error| tier_read(&self.node, error))
    }
}

fn tier(node: &str, error: io::Error) -> I2cError {
    match error.raw_os_error() {
        Some(libc::EACCES) | Some(libc::EPERM) => I2cError::Permission {
            node: node.to_string(),
        },
        Some(libc::ENOENT) => I2cError::NoDevice {
            node: node.to_string(),
        },
        Some(libc::EBUSY) => I2cError::Busy {
            node: node.to_string(),
        },
        Some(libc::EIO) => I2cError::UnsupportedTransport {
            detail: format!(
                "{node}: the DDC/CI transfer failed with EIO; MST branches and DisplayLink links \
                 do not pass DDC/CI"
            ),
        },
        Some(libc::ENXIO) => I2cError::UnsupportedTransport {
            detail: format!("{node}: no DDC/CI device responds on this bus"),
        },
        _ => I2cError::Io(error),
    }
}

fn tier_read(node: &str, error: io::Error) -> I2cError {
    match error.raw_os_error() {
        Some(libc::EACCES) | Some(libc::EPERM) => I2cError::Permission {
            node: node.to_string(),
        },
        Some(libc::ENOENT) => I2cError::NoDevice {
            node: node.to_string(),
        },
        Some(libc::EBUSY) => I2cError::Busy {
            node: node.to_string(),
        },
        _ => I2cError::Io(error),
    }
}
