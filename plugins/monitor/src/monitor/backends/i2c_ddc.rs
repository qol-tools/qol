use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use qol_windowing::DisplayEnumerator;

use crate::monitor::policy::DdcStatus;
use crate::monitor::{
    BrightnessSource, BrightnessState, DisplayCapabilities, DisplayControl, DisplayHandle,
    DisplayMode, GammaState, HdrState, MonitorError, GAMMA_REASON, HDR_REASON, MODES_REASON,
};

const HOST_ADDRESS: u8 = 0x51;
const MONITOR_ADDRESS: u8 = 0x6e;
const OP_GET_VCP: u8 = 0x01;
const OP_GET_VCP_REPLY: u8 = 0x02;
const OP_SET_VCP: u8 = 0x03;
const FEATURE_BRIGHTNESS: u8 = 0x10;
const LENGTH_GET: u8 = 0x82;
const LENGTH_SET: u8 = 0x84;
const LENGTH_REPLY: u8 = 0x88;
const REPLY_VIRTUAL_HOST: u8 = 0x50;
pub(crate) const REPLY_LEN: usize = 11;
pub(crate) const WRITE_RETRIES: usize = 1;
pub(crate) const READ_ATTEMPTS: usize = 20;
pub(crate) const SETTLE_DELAY: Duration = Duration::from_millis(50);
pub(crate) const READ_POLL_DELAY: Duration = Duration::from_millis(10);
pub(crate) const RESPONSE_DELAY: Duration = Duration::from_millis(40);
const ERRNO_EIO: i32 = 5;
const ERRNO_ENXIO: i32 = 6;

#[derive(Debug)]
pub enum I2cError {
    Permission { node: String },
    NoDevice { node: String },
    Busy { node: String },
    UnsupportedTransport { detail: String },
    Protocol { detail: String },
    Io(io::Error),
}

impl fmt::Display for I2cError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Permission { node } => write!(
                f,
                "no permission to access {node}; run `plugin-monitor grant` to apply the i2c \
                 uaccess rule"
            ),
            Self::NoDevice { node } => write!(
                f,
                "no device at {node} (ENOENT); the connector may be unplugged or the i2c-dev \
                 kernel module may not be loaded"
            ),
            Self::Busy { node } => write!(
                f,
                "{node} is busy (EBUSY); a conflicting driver such as ddcci may be holding it"
            ),
            Self::UnsupportedTransport { detail } => write!(f, "{detail}"),
            Self::Protocol { detail } => write!(f, "invalid DDC/CI exchange: {detail}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for I2cError {}

pub trait I2cTransport: Send + Sync {
    type Bus: I2cBus;
    fn open(&self, dev: &Path) -> Result<Self::Bus, I2cError>;
}

pub trait I2cBus {
    fn write(&mut self, frame: &[u8]) -> Result<(), I2cError>;
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, I2cError>;
}

pub struct I2cDdcBackend<T: I2cTransport> {
    transport: T,
    sysfs_drm: PathBuf,
    settle: Duration,
    poll_delay: Duration,
    response_delay: Duration,
    dropped_writes: Mutex<HashSet<String>>,
}

impl<T: I2cTransport> I2cDdcBackend<T> {
    pub fn new(transport: T) -> Self {
        Self::with_timing(
            transport,
            PathBuf::from("/sys/class/drm"),
            SETTLE_DELAY,
            READ_POLL_DELAY,
            RESPONSE_DELAY,
        )
    }

    pub fn with_timing(
        transport: T,
        sysfs_drm: PathBuf,
        settle: Duration,
        poll_delay: Duration,
        response_delay: Duration,
    ) -> Self {
        Self {
            transport,
            sysfs_drm,
            settle,
            poll_delay,
            response_delay,
            dropped_writes: Mutex::new(HashSet::new()),
        }
    }

    fn writes_dropped(&self, connector: &str) -> bool {
        self.dropped_writes.lock().unwrap().contains(connector)
    }

    fn resolve_i2c_dev(&self, connector: &str) -> Result<PathBuf, I2cError> {
        let connector_dir = self.sysfs_drm.join(connector);
        let mut links = Vec::new();
        for entry in fs::read_dir(&connector_dir).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => I2cError::NoDevice {
                node: connector_dir.display().to_string(),
            },
            _ => I2cError::Io(error),
        })? {
            let entry = entry.map_err(I2cError::Io)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_i2c_link_name(&name) {
                links.push((name, entry.path()));
            }
        }
        if links.is_empty() {
            return Err(I2cError::UnsupportedTransport {
                detail: format!(
                    "{connector} exposes no i2c bus; the display transport does not provide \
                     DDC/CI (MST branches and DisplayLink links have no DDC)"
                ),
            });
        }
        links.sort_by_key(|(name, _)| {
            name.strip_prefix("i2c-")
                .and_then(|index| index.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });
        let selected = links
            .iter()
            .find(|(_, path)| self.adapter_is_ddc(path))
            .unwrap_or(&links[0]);
        Ok(PathBuf::from("/dev").join(&selected.0))
    }

    fn adapter_is_ddc(&self, link: &Path) -> bool {
        let Ok(target) = fs::read_link(link) else {
            return false;
        };
        let adapter_dir = if target.is_absolute() {
            target
        } else {
            link.parent().unwrap_or_else(|| Path::new(".")).join(target)
        };
        fs::read_to_string(adapter_dir.join("name"))
            .map(|name| name.to_ascii_lowercase().contains("ddc"))
            .unwrap_or(false)
    }

    fn get_brightness_inner(&self, handle: &DisplayHandle) -> Result<u8, I2cError> {
        let dev = self.resolve_i2c_dev(handle.connector())?;
        let mut bus = self.transport.open(&dev)?;
        let (current, max) = self.read_current_max(&mut bus)?;
        percent_from_raw(current, max)
    }

    fn set_brightness_inner(&self, handle: &DisplayHandle, value: u8) -> Result<(), I2cError> {
        let connector = handle.connector();
        if self.writes_dropped(connector) {
            return Err(I2cError::UnsupportedTransport {
                detail: format!(
                    "DDC/CI writes were dropped on {connector}; set the display policy to gamma \
                     to keep brightness control"
                ),
            });
        }
        let dev = self.resolve_i2c_dev(connector)?;
        let mut bus = self.transport.open(&dev)?;
        let (_, max) = self.read_current_max(&mut bus)?;
        if max == 0 {
            return Err(I2cError::Protocol {
                detail: format!("{connector} reports a maximum brightness of 0"),
            });
        }
        let target = (u32::from(value) * u32::from(max) / 100) as u16;
        let mut verified = false;
        for _ in 0..=WRITE_RETRIES {
            write_set_vcp(&mut bus, target)?;
            thread::sleep(self.settle);
            if self.read_back_matches(&mut bus, value)? {
                verified = true;
                break;
            }
        }
        if !verified {
            self.dropped_writes
                .lock()
                .unwrap()
                .insert(connector.to_string());
        }
        Ok(())
    }

    fn read_back_matches(&self, bus: &mut T::Bus, expected_percent: u8) -> Result<bool, I2cError> {
        let (current, max) = self.read_current_max(bus)?;
        Ok(percent_from_raw(current, max)? == expected_percent)
    }

    fn read_current_max(&self, bus: &mut T::Bus) -> Result<(u16, u16), I2cError> {
        bus.write(&get_vcp_request(FEATURE_BRIGHTNESS))?;
        let frame = self.read_reply(bus)?;
        parse_get_vcp_reply(FEATURE_BRIGHTNESS, &frame)
    }

    fn read_reply(&self, bus: &mut T::Bus) -> Result<[u8; REPLY_LEN], I2cError> {
        thread::sleep(self.response_delay);
        let mut frame = [0u8; REPLY_LEN];
        let mut filled = 0usize;
        let mut attempts = 0usize;
        while filled < REPLY_LEN && attempts < READ_ATTEMPTS {
            match bus.read(&mut frame[filled..]) {
                Ok(0) => {}
                Ok(count) => filled += count,
                Err(I2cError::Io(ref error)) if is_retryable(error) => {}
                Err(error) => return Err(error),
            }
            attempts += 1;
            if filled < REPLY_LEN && attempts < READ_ATTEMPTS {
                thread::sleep(self.poll_delay);
            }
        }
        if filled < REPLY_LEN {
            return Err(I2cError::UnsupportedTransport {
                detail: format!(
                    "no DDC/CI reply after {READ_ATTEMPTS} read attempts; MST branches and \
                     DisplayLink links do not pass DDC/CI"
                ),
            });
        }
        Ok(frame)
    }
}

impl<T: I2cTransport> DisplayControl for I2cDdcBackend<T> {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
        Ok(qol_windowing::Platform.enumerate()?)
    }

    fn probe(&self, handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
        let connector = handle.connector();
        if self.writes_dropped(connector) {
            return Ok(DisplayCapabilities {
                brightness_ddc: false,
                ..DisplayCapabilities::none()
            });
        }
        match self.get_brightness_inner(handle) {
            Ok(_) => Ok(DisplayCapabilities {
                brightness_ddc: true,
                ..DisplayCapabilities::none()
            }),
            Err(error) => match &error {
                I2cError::Permission { .. } | I2cError::NoDevice { .. } | I2cError::Busy { .. } => {
                    Err(error.into())
                }
                _ => Ok(DisplayCapabilities::none()),
            },
        }
    }

    fn get_brightness(&self, handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
        let value = self.get_brightness_inner(handle)?;
        let source = if self.writes_dropped(handle.connector()) {
            BrightnessSource::Gamma
        } else {
            BrightnessSource::Ddc
        };
        Ok(BrightnessState { value, source })
    }

    fn set_brightness(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        self.set_brightness_inner(handle, value)
            .map_err(MonitorError::from)
    }

    fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
        Err(MonitorError::unsupported("gamma", GAMMA_REASON))
    }

    fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("gamma", GAMMA_REASON))
    }

    fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
        Err(MonitorError::unsupported("modes", MODES_REASON))
    }

    fn set_mode(&self, _handle: &DisplayHandle, _mode: &DisplayMode) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("modes", MODES_REASON))
    }

    fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
        Err(MonitorError::unsupported("hdr", HDR_REASON))
    }

    fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("hdr", HDR_REASON))
    }
}

fn is_i2c_link_name(name: &str) -> bool {
    name.strip_prefix("i2c-")
        .is_some_and(|index| !index.is_empty() && index.chars().all(|c| c.is_ascii_digit()))
}

fn is_retryable(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(ERRNO_EIO) | Some(ERRNO_ENXIO))
}

pub(crate) fn xor_checksum(seed: u8, bytes: &[u8]) -> u8 {
    bytes.iter().fold(seed, |acc, byte| acc ^ byte)
}

pub(crate) fn get_vcp_request(feature: u8) -> [u8; 5] {
    let mut frame = [HOST_ADDRESS, LENGTH_GET, OP_GET_VCP, feature, 0];
    frame[4] = xor_checksum(MONITOR_ADDRESS, &frame[..4]);
    frame
}

pub(crate) fn set_vcp_request(feature: u8, value: u16) -> [u8; 7] {
    let mut frame = [
        HOST_ADDRESS,
        LENGTH_SET,
        OP_SET_VCP,
        feature,
        (value >> 8) as u8,
        value as u8,
        0,
    ];
    frame[6] = xor_checksum(MONITOR_ADDRESS, &frame[..6]);
    frame
}

fn write_set_vcp(bus: &mut impl I2cBus, raw: u16) -> Result<(), I2cError> {
    bus.write(&set_vcp_request(FEATURE_BRIGHTNESS, raw))
}

pub(crate) fn parse_get_vcp_reply(
    feature: u8,
    frame: &[u8; REPLY_LEN],
) -> Result<(u16, u16), I2cError> {
    if xor_checksum(REPLY_VIRTUAL_HOST, &frame[1..10]) != frame[10] {
        return Err(I2cError::Protocol {
            detail: "reply checksum mismatch".into(),
        });
    }
    if frame[0] != MONITOR_ADDRESS || frame[1] != LENGTH_REPLY {
        return Err(I2cError::Protocol {
            detail: "reply carries the wrong source or destination address".into(),
        });
    }
    if frame[2] != OP_GET_VCP_REPLY {
        return Err(I2cError::Protocol {
            detail: "expected a get-vcp-feature reply".into(),
        });
    }
    if frame[4] != feature {
        return Err(I2cError::Protocol {
            detail: format!(
                "reply carries feature 0x{:02x}, expected 0x{feature:02x}",
                frame[4]
            ),
        });
    }
    match frame[3] {
        0x00 => {}
        0x01 => {
            return Err(I2cError::Protocol {
                detail: format!("the display does not support feature 0x{feature:02x}"),
            })
        }
        code => {
            return Err(I2cError::Protocol {
                detail: format!("reply result code 0x{code:02x}"),
            })
        }
    }
    Ok((
        u16::from_be_bytes([frame[8], frame[9]]),
        u16::from_be_bytes([frame[6], frame[7]]),
    ))
}

pub(crate) fn percent_from_raw(current: u16, max: u16) -> Result<u8, I2cError> {
    if max == 0 {
        return Err(I2cError::Protocol {
            detail: "the display reports a maximum brightness of 0".into(),
        });
    }
    let percent = u32::from(current) * 100 / u32::from(max);
    Ok(percent.min(100) as u8)
}

impl<T: I2cTransport> DdcStatus for I2cDdcBackend<T> {
    fn writes_dropped(&self, connector: &str) -> bool {
        self.writes_dropped(connector)
    }
}

#[cfg(target_os = "linux")]
const I2C_SLAVE_REQUEST: libc::c_ulong = 0x0703;
#[cfg(target_os = "linux")]
const DDC_CI_SLAVE_ADDRESS: u8 = 0x37;

#[cfg(target_os = "linux")]
pub struct LinuxI2cTransport;

#[cfg(target_os = "linux")]
pub struct I2cFileBus {
    file: std::fs::File,
    node: String,
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::monitor::{BrightnessSource, DisplayCapabilities};

    #[derive(Clone)]
    enum FakeFailure {
        Permission,
        NoDevice,
        Busy,
        Unsupported,
        Eio,
    }

    impl FakeFailure {
        fn to_i2c_error(&self) -> I2cError {
            match self {
                Self::Permission => I2cError::Permission {
                    node: "/dev/i2c-test".into(),
                },
                Self::NoDevice => I2cError::NoDevice {
                    node: "/dev/i2c-test".into(),
                },
                Self::Busy => I2cError::Busy {
                    node: "/dev/i2c-test".into(),
                },
                Self::Unsupported => I2cError::UnsupportedTransport {
                    detail: "the fake transport does not support DDC/CI".into(),
                },
                Self::Eio => I2cError::Io(io::Error::from_raw_os_error(ERRNO_EIO)),
            }
        }
    }

    struct FakeMonitor {
        current: u16,
        max: u16,
        drop_writes: bool,
        write_failure: Option<FakeFailure>,
        read_failure: Option<FakeFailure>,
        frames_written: Vec<Vec<u8>>,
        opens: Vec<String>,
        pending_reply: Option<Vec<u8>>,
    }

    impl FakeMonitor {
        fn new(current: u16, max: u16) -> Self {
            Self {
                current,
                max,
                drop_writes: false,
                write_failure: None,
                read_failure: None,
                frames_written: Vec::new(),
                opens: Vec::new(),
                pending_reply: None,
            }
        }
    }

    struct FakeTransport {
        monitor: Arc<Mutex<FakeMonitor>>,
        open_failure: Option<FakeFailure>,
    }

    impl FakeTransport {
        fn new(monitor: FakeMonitor) -> Self {
            Self {
                monitor: Arc::new(Mutex::new(monitor)),
                open_failure: None,
            }
        }
    }

    struct FakeBus {
        monitor: Arc<Mutex<FakeMonitor>>,
    }

    impl I2cTransport for FakeTransport {
        type Bus = FakeBus;

        fn open(&self, dev: &Path) -> Result<Self::Bus, I2cError> {
            if let Some(failure) = &self.open_failure {
                return Err(failure.to_i2c_error());
            }
            self.monitor
                .lock()
                .unwrap()
                .opens
                .push(dev.display().to_string());
            Ok(FakeBus {
                monitor: Arc::clone(&self.monitor),
            })
        }
    }

    impl I2cBus for FakeBus {
        fn write(&mut self, frame: &[u8]) -> Result<(), I2cError> {
            let mut monitor = self.monitor.lock().unwrap();
            if let Some(failure) = &monitor.write_failure {
                return Err(failure.to_i2c_error());
            }
            monitor.frames_written.push(frame.to_vec());
            if frame[1] != LENGTH_GET && frame[1] != LENGTH_SET {
                return Err(I2cError::Protocol {
                    detail: "request carries an unknown DDC/CI length byte".into(),
                });
            }
            if xor_checksum(MONITOR_ADDRESS, &frame[..frame.len() - 1]) != frame[frame.len() - 1] {
                return Err(I2cError::Protocol {
                    detail: "request checksum mismatch".into(),
                });
            }
            match frame[2] {
                OP_SET_VCP => {
                    let value = u16::from_be_bytes([frame[4], frame[5]]);
                    if !monitor.drop_writes {
                        monitor.current = value;
                    }
                }
                OP_GET_VCP => {
                    let mut reply = [0u8; REPLY_LEN];
                    reply[0] = MONITOR_ADDRESS;
                    reply[1] = LENGTH_REPLY;
                    reply[2] = OP_GET_VCP_REPLY;
                    reply[4] = frame[3];
                    reply[6] = (monitor.max >> 8) as u8;
                    reply[7] = monitor.max as u8;
                    reply[8] = (monitor.current >> 8) as u8;
                    reply[9] = monitor.current as u8;
                    reply[10] = xor_checksum(REPLY_VIRTUAL_HOST, &reply[1..10]);
                    monitor.pending_reply = Some(reply.to_vec());
                }
                _ => {
                    return Err(I2cError::Protocol {
                        detail: "unknown DDC/CI opcode".into(),
                    })
                }
            }
            Ok(())
        }

        fn read(&mut self, buffer: &mut [u8]) -> Result<usize, I2cError> {
            let mut monitor = self.monitor.lock().unwrap();
            if let Some(failure) = &monitor.read_failure {
                return Err(failure.to_i2c_error());
            }
            let Some(pending) = monitor.pending_reply.take() else {
                return Ok(0);
            };
            let count = pending.len().min(buffer.len());
            buffer[..count].copy_from_slice(&pending[..count]);
            Ok(count)
        }
    }

    fn handle() -> DisplayHandle {
        DisplayHandle::new("id-1".into(), "card0-DP-1".into(), None, false)
    }

    fn sysfs_with_links(links: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let connector_dir = dir.path().join("card0-DP-1");
        fs::create_dir_all(&connector_dir).unwrap();
        let adapters = dir.path().join("adapters");
        fs::create_dir_all(&adapters).unwrap();
        for (link, name) in links {
            let adapter = adapters.join(link);
            fs::create_dir_all(&adapter).unwrap();
            fs::write(adapter.join("name"), name).unwrap();
            std::os::unix::fs::symlink(format!("../adapters/{link}"), connector_dir.join(link))
                .unwrap();
        }
        dir
    }

    fn backend(monitor: FakeMonitor, sysfs: PathBuf) -> I2cDdcBackend<FakeTransport> {
        I2cDdcBackend::with_timing(
            FakeTransport::new(monitor),
            sysfs,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
        )
    }

    #[test]
    fn get_vcp_request_matches_the_canonical_ddc_ci_bytes() {
        assert_eq!(
            get_vcp_request(FEATURE_BRIGHTNESS),
            [0x51, 0x82, 0x01, 0x10, 0xac]
        );
    }

    #[test]
    fn set_vcp_request_matches_the_canonical_ddc_ci_bytes() {
        assert_eq!(
            set_vcp_request(FEATURE_BRIGHTNESS, 0x0032),
            [0x51, 0x84, 0x03, 0x10, 0x00, 0x32, 0x9a]
        );
        assert_eq!(
            set_vcp_request(FEATURE_BRIGHTNESS, 500),
            [0x51, 0x84, 0x03, 0x10, 0x01, 0xf4, 0x5d]
        );
    }

    #[test]
    fn parse_get_vcp_reply_accepts_a_valid_frame_and_rejects_corruption() {
        let mut frame = [0u8; REPLY_LEN];
        frame[0] = MONITOR_ADDRESS;
        frame[1] = LENGTH_REPLY;
        frame[2] = OP_GET_VCP_REPLY;
        frame[4] = FEATURE_BRIGHTNESS;
        frame[6] = 0x03;
        frame[7] = 0xe8;
        frame[8] = 0x01;
        frame[9] = 0xf4;
        frame[10] = xor_checksum(REPLY_VIRTUAL_HOST, &frame[1..10]);
        assert_eq!(
            parse_get_vcp_reply(FEATURE_BRIGHTNESS, &frame).unwrap(),
            (500, 1000)
        );

        let mut bad_checksum = frame;
        bad_checksum[10] = bad_checksum[10].wrapping_add(1);
        assert!(matches!(
            parse_get_vcp_reply(FEATURE_BRIGHTNESS, &bad_checksum),
            Err(I2cError::Protocol { .. })
        ));

        let mut wrong_length = frame;
        wrong_length[1] = LENGTH_GET;
        wrong_length[10] = xor_checksum(REPLY_VIRTUAL_HOST, &wrong_length[1..10]);
        assert!(matches!(
            parse_get_vcp_reply(FEATURE_BRIGHTNESS, &wrong_length),
            Err(I2cError::Protocol { .. })
        ));

        let mut wrong_feature = frame;
        wrong_feature[4] = 0x12;
        wrong_feature[10] = xor_checksum(REPLY_VIRTUAL_HOST, &wrong_feature[1..10]);
        assert!(matches!(
            parse_get_vcp_reply(FEATURE_BRIGHTNESS, &wrong_feature),
            Err(I2cError::Protocol { .. })
        ));

        let mut unsupported_feature = frame;
        unsupported_feature[3] = 0x01;
        unsupported_feature[10] = xor_checksum(REPLY_VIRTUAL_HOST, &unsupported_feature[1..10]);
        assert!(matches!(
            parse_get_vcp_reply(FEATURE_BRIGHTNESS, &unsupported_feature),
            Err(I2cError::Protocol { .. })
        ));
    }

    #[test]
    fn percent_from_raw_converts_within_the_reported_range() {
        assert_eq!(percent_from_raw(0, 1000).unwrap(), 0);
        assert_eq!(percent_from_raw(250, 1000).unwrap(), 25);
        assert_eq!(percent_from_raw(999, 1000).unwrap(), 99);
        assert_eq!(percent_from_raw(1000, 1000).unwrap(), 100);
        assert_eq!(percent_from_raw(1000, 500).unwrap(), 100);
    }

    #[test]
    fn percent_from_raw_rejects_a_zero_maximum() {
        assert!(matches!(
            percent_from_raw(0, 0),
            Err(I2cError::Protocol { .. })
        ));
    }

    #[test]
    fn unsupported_transport_maps_to_typed_unsupported() {
        let error = MonitorError::from(I2cError::UnsupportedTransport {
            detail: "no i2c bus".into(),
        });
        match error {
            MonitorError::Unsupported { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert_eq!(reason, "no i2c bus");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn get_reads_brightness_over_the_resolved_i2c_bus() {
        let sysfs = sysfs_with_links(&[("i2c-7", "i915 gmbus dp ddc")]);
        let backend = backend(FakeMonitor::new(200, 1000), sysfs.path().to_path_buf());
        let state = backend.get_brightness(&handle()).unwrap();
        assert_eq!(state.value, 20);
        assert_eq!(state.source, BrightnessSource::Ddc);
    }

    #[test]
    fn set_writes_the_raw_value_and_verifies_by_read_back() {
        let sysfs = sysfs_with_links(&[("i2c-7", "i915 gmbus dp ddc")]);
        let backend = backend(FakeMonitor::new(200, 1000), sysfs.path().to_path_buf());
        backend.set_brightness(&handle(), 50).unwrap();
        let state = backend.get_brightness(&handle()).unwrap();
        assert_eq!(state.value, 50);
        assert_eq!(state.source, BrightnessSource::Ddc);
        let monitor = backend.transport.monitor.lock().unwrap();
        assert_eq!(
            monitor.opens,
            vec!["/dev/i2c-7".to_string(), "/dev/i2c-7".to_string()],
            "set and get each open the resolved bus"
        );
        assert_eq!(
            monitor.frames_written.len(),
            4,
            "set pre-read, set, verify read-back, then get"
        );
        assert_eq!(
            monitor.frames_written[1].as_slice(),
            &[0x51, 0x84, 0x03, 0x10, 0x01, 0xf4, 0x5d]
        );
    }

    #[test]
    fn probe_reports_ddc_when_a_brightness_read_succeeds() {
        let sysfs = sysfs_with_links(&[("i2c-7", "i915 gmbus dp ddc")]);
        let backend = backend(FakeMonitor::new(200, 1000), sysfs.path().to_path_buf());
        let caps = backend.probe(&handle()).unwrap();
        assert_eq!(
            caps,
            DisplayCapabilities {
                brightness_ddc: true,
                ..DisplayCapabilities::none()
            }
        );
    }

    #[test]
    fn dropped_writes_downgrade_the_source_and_disable_the_capability() {
        let sysfs = sysfs_with_links(&[("i2c-7", "i915 gmbus dp ddc")]);
        let monitor = FakeMonitor {
            drop_writes: true,
            ..FakeMonitor::new(200, 1000)
        };
        let backend = backend(monitor, sysfs.path().to_path_buf());
        backend.set_brightness(&handle(), 50).unwrap();
        {
            let monitor = backend.transport.monitor.lock().unwrap();
            assert_eq!(
                monitor.frames_written.len(),
                5,
                "pre-read, two set attempts, two verify read-backs"
            );
        }
        let state = backend.get_brightness(&handle()).unwrap();
        assert_eq!(state.value, 20);
        assert_eq!(state.source, BrightnessSource::Gamma);
        let caps = backend.probe(&handle()).unwrap();
        assert!(!caps.brightness_ddc);
        let error = backend.set_brightness(&handle(), 60).unwrap_err();
        match error {
            MonitorError::Unsupported { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert!(reason.contains("writes were dropped"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn eio_write_maps_to_typed_unsupported() {
        let sysfs = sysfs_with_links(&[("i2c-7", "i915 gmbus dp ddc")]);
        let monitor = FakeMonitor {
            write_failure: Some(FakeFailure::Unsupported),
            ..FakeMonitor::new(200, 1000)
        };
        let backend = backend(monitor, sysfs.path().to_path_buf());
        let error = backend.set_brightness(&handle(), 50).unwrap_err();
        assert!(matches!(
            error,
            MonitorError::Unsupported {
                capability: "brightness",
                ..
            }
        ));
    }

    #[test]
    fn persistent_eio_reads_map_to_typed_unsupported() {
        let sysfs = sysfs_with_links(&[("i2c-7", "i915 gmbus dp ddc")]);
        let monitor = FakeMonitor {
            read_failure: Some(FakeFailure::Eio),
            ..FakeMonitor::new(200, 1000)
        };
        let backend = backend(monitor, sysfs.path().to_path_buf());
        let error = backend.get_brightness(&handle()).unwrap_err();
        match error {
            MonitorError::Unsupported { capability, reason } => {
                assert_eq!(capability, "brightness");
                assert!(reason.contains("no DDC/CI reply"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn connector_without_an_i2c_link_is_unsupported() {
        let sysfs = sysfs_with_links(&[]);
        let backend = backend(FakeMonitor::new(200, 1000), sysfs.path().to_path_buf());
        let caps = backend.probe(&handle()).unwrap();
        assert!(!caps.brightness_ddc);
        let error = backend.get_brightness(&handle()).unwrap_err();
        assert!(matches!(
            error,
            MonitorError::Unsupported {
                capability: "brightness",
                ..
            }
        ));
    }

    #[test]
    fn open_tiers_surface_at_the_facade() {
        for (failure, needle) in [
            (FakeFailure::Permission, "plugin-monitor grant"),
            (FakeFailure::NoDevice, "i2c-dev"),
            (FakeFailure::Busy, "ddcci"),
        ] {
            let sysfs = sysfs_with_links(&[("i2c-7", "i915 gmbus dp ddc")]);
            let mut transport = FakeTransport::new(FakeMonitor::new(200, 1000));
            transport.open_failure = Some(failure);
            let backend = I2cDdcBackend::with_timing(
                transport,
                sysfs.path().to_path_buf(),
                Duration::ZERO,
                Duration::ZERO,
                Duration::ZERO,
            );
            let error = backend.get_brightness(&handle()).unwrap_err();
            match error {
                MonitorError::I2c(error) => {
                    assert!(error.to_string().contains(needle), "{error}")
                }
                other => panic!("expected I2c error, got {other:?}"),
            }
        }
    }

    #[test]
    fn probe_surfaces_open_tiers_as_errors() {
        let sysfs = sysfs_with_links(&[("i2c-7", "i915 gmbus dp ddc")]);
        let mut transport = FakeTransport::new(FakeMonitor::new(200, 1000));
        transport.open_failure = Some(FakeFailure::Permission);
        let backend = I2cDdcBackend::with_timing(
            transport,
            sysfs.path().to_path_buf(),
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
        );
        assert!(matches!(
            backend.probe(&handle()),
            Err(MonitorError::I2c(I2cError::Permission { .. }))
        ));
    }

    #[test]
    fn resolution_prefers_the_ddc_named_adapter() {
        let sysfs =
            sysfs_with_links(&[("i2c-3", "i915 gmbus aux"), ("i2c-9", "i915 gmbus dp ddc")]);
        let backend = backend(FakeMonitor::new(200, 1000), sysfs.path().to_path_buf());
        backend.get_brightness(&handle()).unwrap();
        let monitor = backend.transport.monitor.lock().unwrap();
        assert_eq!(monitor.opens, vec!["/dev/i2c-9".to_string()]);
    }

    #[test]
    fn resolution_falls_back_to_the_lowest_adapter_number() {
        let sysfs = sysfs_with_links(&[("i2c-9", "aux"), ("i2c-3", "aux")]);
        let backend = backend(FakeMonitor::new(200, 1000), sysfs.path().to_path_buf());
        backend.get_brightness(&handle()).unwrap();
        let monitor = backend.transport.monitor.lock().unwrap();
        assert_eq!(monitor.opens, vec!["/dev/i2c-3".to_string()]);
    }

    #[test]
    fn resolution_falls_back_to_the_lowest_numeric_adapter_number() {
        let sysfs = sysfs_with_links(&[("i2c-9", "aux"), ("i2c-10", "aux"), ("i2c-3", "aux")]);
        let backend = backend(FakeMonitor::new(200, 1000), sysfs.path().to_path_buf());
        backend.get_brightness(&handle()).unwrap();
        let monitor = backend.transport.monitor.lock().unwrap();
        assert_eq!(
            monitor.opens,
            vec!["/dev/i2c-3".to_string()],
            "i2c-10 must not sort before i2c-3 lexicographically"
        );
    }
}
