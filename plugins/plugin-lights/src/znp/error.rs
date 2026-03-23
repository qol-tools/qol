use std::fmt;

#[derive(Debug)]
pub enum ZnpError {
    SerialPort(String),
    FrameDecode(String),
    Timeout,
    NotConnected,
    CommandFailed { subsystem: u8, cmd: u8, status: u8 },
}

impl fmt::Display for ZnpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SerialPort(msg) => write!(f, "serial port error: {}", msg),
            Self::FrameDecode(msg) => write!(f, "frame decode error: {}", msg),
            Self::Timeout => write!(f, "ZNP request timed out"),
            Self::NotConnected => write!(f, "not connected to coordinator"),
            Self::CommandFailed { subsystem, cmd, status } => {
                write!(f, "command failed: subsystem=0x{:02X} cmd=0x{:02X} status=0x{:02X}", subsystem, cmd, status)
            }
        }
    }
}

impl std::error::Error for ZnpError {}
