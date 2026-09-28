use crate::config::ServerConfig;
use if_addrs::get_if_addrs;
use std::net::IpAddr;

pub(crate) struct InterfaceMetadata {
    pub name: String,
    pub address: IpAddr,
    pub loopback: bool,
}

pub(crate) struct NetworkMetadata {
    pub hostname: String,
    pub local_ipv4: Option<IpAddr>,
    pub interfaces: Vec<InterfaceMetadata>,
    pub interface_issue: Option<String>,
}

pub fn get_local_ip() -> Option<IpAddr> {
    get_if_addrs()
        .ok()?
        .iter()
        .find(|iface| !iface.is_loopback() && iface.ip().is_ipv4())
        .map(|iface| iface.ip())
}

pub fn get_hostname() -> String {
    gethostname::gethostname()
        .into_string()
        .unwrap_or_else(|_| ServerConfig::UNKNOWN_HOSTNAME.to_string())
}

pub(crate) fn inspect_metadata() -> NetworkMetadata {
    let hostname = get_hostname();
    let interfaces = match get_if_addrs() {
        Ok(interfaces) => interfaces,
        Err(error) => {
            return NetworkMetadata {
                hostname,
                local_ipv4: None,
                interfaces: Vec::new(),
                interface_issue: Some(error.to_string()),
            };
        }
    };
    let local_ipv4 = interfaces
        .iter()
        .find(|interface| !interface.is_loopback() && interface.ip().is_ipv4())
        .map(|interface| interface.ip());
    let mut interfaces = interfaces
        .into_iter()
        .map(|interface| {
            let address = interface.ip();
            let loopback = interface.is_loopback();
            InterfaceMetadata {
                name: interface.name,
                address,
                loopback,
            }
        })
        .collect::<Vec<_>>();
    interfaces.sort_by(|left, right| {
        (&left.name, left.address.to_string()).cmp(&(&right.name, right.address.to_string()))
    });
    NetworkMetadata {
        hostname,
        local_ipv4,
        interfaces,
        interface_issue: None,
    }
}
