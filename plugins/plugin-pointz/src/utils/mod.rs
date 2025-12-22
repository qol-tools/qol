use std::net::IpAddr;
use if_addrs::get_if_addrs;
use crate::domain::config::ServerConfig;

pub fn get_local_ip() -> Option<IpAddr> {
    get_if_addrs()
        .ok()?
        .iter()
        .find(|iface| !iface.is_loopback() && iface.ip().is_ipv4())
        .map(|iface| iface.ip())
}

pub fn get_hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| ServerConfig::UNKNOWN_HOSTNAME.to_string())
}

