use serialport::SerialPortInfo;

pub(crate) fn detect_coordinator_port(_ports: &[SerialPortInfo]) -> Option<String> {
    None
}

pub(crate) fn candidate_coordinator_ports(_ports: &[SerialPortInfo]) -> Vec<String> {
    Vec::new()
}
