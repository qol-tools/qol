use anyhow::Result;

use super::request::RequestEngine;
use super::transport::{Transport, TransportConfig};

pub fn detect_coordinator_port() -> Option<String> {
    let ports = serialport::available_ports().ok()?;
    crate::platform::detect_coordinator_port(&ports)
}

pub fn candidate_coordinator_ports() -> Vec<String> {
    serialport::available_ports()
        .map(|ports| crate::platform::candidate_coordinator_ports(&ports))
        .unwrap_or_default()
}

pub fn probe_candidate_coordinator_ports(ports: &[String]) -> Option<String> {
    ports
        .iter()
        .find(|port| probe_coordinator_port(port).is_ok())
        .cloned()
}

pub fn available_port_descriptions() -> Vec<String> {
    serialport::available_ports()
        .map(|ports| {
            ports
                .into_iter()
                .map(|port| crate::platform::describe_port(&port))
                .collect()
        })
        .unwrap_or_default()
}

fn probe_coordinator_port(port: &str) -> Result<()> {
    let transport = Transport::open(&TransportConfig {
        port: port.to_string(),
        baud_rate: 115_200,
    })?;
    let engine = RequestEngine::new(transport);
    super::coordinator::probe(&engine)
}
