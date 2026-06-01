use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Device {
    pub network_address: u16,
    pub ieee_address: [u8; 8],
    pub endpoints: Vec<Endpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Endpoint {
    pub id: u8,
    pub input_clusters: Vec<u16>,
}

impl Device {
    pub fn endpoint_for_cluster(&self, cluster_id: u16) -> Option<u8> {
        self.endpoints
            .iter()
            .find(|ep| ep.input_clusters.contains(&cluster_id))
            .map(|ep| ep.id)
    }
}

pub struct DeviceRegistry {
    devices: Vec<Device>,
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceRegistry {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
        }
    }

    pub fn from_persisted(devices: Vec<Device>) -> Self {
        Self { devices }
    }

    pub fn register(&mut self, device: Device) {
        if let Some(existing) = self
            .devices
            .iter_mut()
            .find(|d| d.ieee_address == device.ieee_address)
        {
            existing.network_address = device.network_address;
            if !device.endpoints.is_empty() {
                existing.endpoints = device.endpoints;
            }
        } else {
            self.devices.push(device);
        }
    }

    pub fn remove(&mut self, ieee_address: &[u8; 8]) {
        self.devices.retain(|d| &d.ieee_address != ieee_address);
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn devices_mut(&mut self) -> &mut [Device] {
        &mut self.devices
    }

    pub fn by_network_address(&self, addr: u16) -> Option<&Device> {
        self.devices.iter().find(|d| d.network_address == addr)
    }

    pub fn by_ieee_address(&self, ieee: &[u8; 8]) -> Option<&Device> {
        self.devices.iter().find(|d| &d.ieee_address == ieee)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_device(network_address: u16, ieee_last_byte: u8) -> Device {
        Device {
            network_address,
            ieee_address: [0, 0, 0, 0, 0, 0, 0, ieee_last_byte],
            endpoints: vec![Endpoint {
                id: 1,
                input_clusters: vec![0x0006],
            }],
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut registry = DeviceRegistry::new();
        registry.register(make_device(0x1234, 1));
        assert_eq!(registry.devices().len(), 1);
        let found = registry.by_network_address(0x1234);
        assert!(found.is_some(), "expected device at 0x1234");
        assert_eq!(found.unwrap().network_address, 0x1234);
    }

    #[test]
    fn update_existing_device_by_ieee() {
        let mut registry = DeviceRegistry::new();
        registry.register(make_device(0x1234, 1));
        registry.register(make_device(0x5678, 1));
        assert_eq!(
            registry.devices().len(),
            1,
            "expected single device after update"
        );
        assert_eq!(registry.devices()[0].network_address, 0x5678);
    }

    #[test]
    fn remove_device() {
        let mut registry = DeviceRegistry::new();
        let ieee = [0u8; 8];
        let device = Device {
            network_address: 0x0001,
            ieee_address: ieee,
            endpoints: vec![],
        };
        registry.register(device);
        assert_eq!(registry.devices().len(), 1);
        registry.remove(&ieee);
        assert_eq!(
            registry.devices().len(),
            0,
            "expected empty registry after remove"
        );
    }

    #[test]
    fn persistence_round_trip() {
        let devices = vec![make_device(0xABCD, 42)];
        let registry = DeviceRegistry::from_persisted(devices.clone());
        let json = serde_json::to_string(registry.devices()).unwrap();
        let restored: Vec<Device> = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, devices, "devices must survive JSON round-trip");
    }

    #[test]
    fn has_cluster() {
        let device = Device {
            network_address: 0x0001,
            ieee_address: [0u8; 8],
            endpoints: vec![
                Endpoint {
                    id: 1,
                    input_clusters: vec![0x0006, 0x0008],
                },
                Endpoint {
                    id: 2,
                    input_clusters: vec![0x0300],
                },
            ],
        };

        let cases: &[(u16, Option<u8>)] = &[
            (0x0006, Some(1)),
            (0x0008, Some(1)),
            (0x0300, Some(2)),
            (0x0000, None),
        ];

        for (cluster_id, expected) in cases {
            assert_eq!(
                device.endpoint_for_cluster(*cluster_id),
                *expected,
                "cluster_id: {:#06x}",
                cluster_id
            );
        }
    }
}
