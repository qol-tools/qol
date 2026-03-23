const SONOFF_VID: u16 = 0x10C4;
const SONOFF_PID: u16 = 0xEA60;

pub fn detect_sonoff() -> Option<String> {
    let ports = serialport::available_ports().ok()?;
    ports.iter().find_map(|port| {
        if let serialport::SerialPortType::UsbPort(usb) = &port.port_type {
            if usb.vid == SONOFF_VID && usb.pid == SONOFF_PID {
                return Some(port.port_name.clone());
            }
        }
        None
    })
}
