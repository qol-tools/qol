pub(crate) mod client;

pub(crate) const PQRS_SOCKET: &str =
    "/Library/Application Support/org.pqrs/tmp/rootonly/karabiner_virtual_hid_device_service.sock";
pub(crate) const PQRS_DAEMON_BINARY: &str = "/Library/Application Support/org.pqrs/Karabiner-DriverKit-VirtualHIDDevice/Applications/Karabiner-VirtualHIDDevice-Daemon.app/Contents/MacOS/Karabiner-VirtualHIDDevice-Daemon";
pub(crate) const PQRS_MANAGER_BINARY: &str =
    "/Applications/.Karabiner-VirtualHIDDevice-Manager.app/Contents/MacOS/Karabiner-VirtualHIDDevice-Manager";
pub(crate) const PQRS_DAEMON_LABEL: &str =
    "org.pqrs.service.daemon.Karabiner-VirtualHIDDevice-Daemon";
pub(crate) const OWN_DAEMON_LABEL: &str = "com.qol-tools.keyremap.vhid-daemon";
