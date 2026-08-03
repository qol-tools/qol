pub fn detect_coordinator_port() -> Option<String> {
    let metadata = crate::platform::enumerate_serial_metadata().ok()?;
    crate::platform::detect_coordinator_port(&metadata.ports)
}

pub fn candidate_coordinator_ports() -> Vec<String> {
    crate::platform::enumerate_serial_metadata()
        .map(|metadata| crate::platform::candidate_coordinator_ports(&metadata.ports))
        .unwrap_or_default()
}

pub fn available_port_descriptions() -> Vec<String> {
    crate::platform::enumerate_serial_metadata()
        .map(|metadata| {
            metadata
                .ports
                .into_iter()
                .map(|port| crate::platform::describe_port(&port))
                .collect()
        })
        .unwrap_or_default()
}
