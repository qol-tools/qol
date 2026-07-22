use serialport::{SerialPortInfo, SerialPortType, UsbPortInfo};

pub(crate) fn describe_port(port: &SerialPortInfo) -> String {
    let Some(usb) = usb_port(port) else {
        return port.port_name.clone();
    };

    let mut parts = Vec::new();
    if let Some(manufacturer) = &usb.manufacturer {
        let manufacturer = manufacturer.trim();
        if !manufacturer.is_empty() {
            parts.push(manufacturer.to_string());
        }
    }
    if let Some(product) = &usb.product {
        let product = product.trim();
        if !product.is_empty() {
            parts.push(product.to_string());
        }
    }
    parts.push(format!("{:04x}:{:04x}", usb.vid, usb.pid));

    format!("{} [{}]", port.port_name, parts.join(" · "))
}

pub(super) fn usb_port(port: &SerialPortInfo) -> Option<&UsbPortInfo> {
    let SerialPortType::UsbPort(usb) = &port.port_type else {
        return None;
    };

    Some(usb)
}
