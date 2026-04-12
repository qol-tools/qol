use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};

use super::coordinator::{self, NetworkConfig};
use super::device::{Device, DeviceRegistry, Endpoint};
use super::frame::ZnpFrame;
use super::request::RequestEngine;
use super::subsystem;
use super::transport::{Transport, TransportConfig};
use super::zcl::ZclFrame;

const EVENT_LOOP_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ControllerConfig {
    pub serial_port: String,
    pub baud_rate: u32,
    pub network_key: [u8; 16],
    pub channel: u8,
    pub pan_id: u16,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            serial_port: String::new(),
            baud_rate: 115_200,
            network_key: [
                0x01, 0x03, 0x05, 0x07, 0x09, 0x0B, 0x0D, 0x0F, 0x00, 0x02, 0x04, 0x06, 0x08, 0x0A,
                0x0C, 0x0D,
            ],
            channel: 11,
            pan_id: 0x1A62,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ZigbeeEvent {
    DeviceJoined(Device),
    DeviceLeft([u8; 8]),
}

pub struct ZigbeeController {
    engine: Arc<RequestEngine>,
    registry: Arc<Mutex<DeviceRegistry>>,
    events_rx: Receiver<ZigbeeEvent>,
    _event_loop: thread::JoinHandle<()>,
}

impl ZigbeeController {
    pub fn open(config: ControllerConfig, persisted_devices: Vec<Device>) -> Result<Self> {
        let transport = Transport::open(&TransportConfig {
            port: config.serial_port.clone(),
            baud_rate: config.baud_rate,
        })
        .context("failed to open serial transport")?;

        let engine = Arc::new(RequestEngine::new(transport));

        let network_config = NetworkConfig {
            pan_id: config.pan_id,
            channel: config.channel,
            network_key: config.network_key,
        };
        coordinator::startup(&engine, &network_config).context("coordinator startup failed")?;

        let registry = Arc::new(Mutex::new(DeviceRegistry::from_persisted(
            persisted_devices,
        )));
        let (events_tx, events_rx) = bounded(64);

        let loop_engine = Arc::clone(&engine);
        let loop_registry = Arc::clone(&registry);
        let event_loop = thread::Builder::new()
            .name("zigbee-event-loop".into())
            .spawn(move || run_event_loop(loop_engine, loop_registry, events_tx))
            .context("failed to spawn event loop thread")?;

        Ok(Self {
            engine,
            registry,
            events_rx,
            _event_loop: event_loop,
        })
    }

    pub fn permit_join(&self, duration_secs: u8) -> Result<()> {
        coordinator::permit_join(&self.engine, duration_secs)
    }

    pub fn devices(&self) -> Vec<Device> {
        self.registry
            .lock()
            .map(|r| r.devices().to_vec())
            .unwrap_or_default()
    }

    pub fn send_cluster_command(
        &self,
        network_address: u16,
        endpoint: u8,
        cluster_id: u16,
        zcl_frame: ZclFrame,
    ) -> Result<()> {
        let payload = zcl_frame.encode();
        eprintln!(
            "[znp] send ZCL: addr=0x{:04X} ep={} cluster=0x{:04X} payload={:02X?}",
            network_address, endpoint, cluster_id, payload
        );
        coordinator::send_zcl_command(
            &self.engine,
            network_address,
            endpoint,
            cluster_id,
            &payload,
        )
    }

    pub fn resolve_nwk_address(&self, ieee: &[u8; 8]) -> Result<u16> {
        let nwk = coordinator::resolve_nwk_address(&self.engine, ieee)?;
        let mut reg = self.registry.lock().unwrap();
        if let Some(dev) = reg.by_ieee_address(ieee).cloned() {
            if dev.network_address != nwk {
                eprintln!(
                    "[znp] device {} NWK updated: 0x{:04X} → 0x{:04X}",
                    format_ieee(&dev.ieee_address), dev.network_address, nwk,
                );
                let mut updated = dev;
                updated.network_address = nwk;
                reg.register(updated);
            }
        }
        Ok(nwk)
    }

    pub fn events(&self) -> &Receiver<ZigbeeEvent> {
        &self.events_rx
    }
}

fn run_event_loop(
    engine: Arc<RequestEngine>,
    registry: Arc<Mutex<DeviceRegistry>>,
    events_tx: Sender<ZigbeeEvent>,
) {
    while let Ok(frame) = engine.events().recv() {
        if frame.subsystem() == subsystem::ZDO && frame.cmd1 == subsystem::zdo::END_DEVICE_ANNCE_IND
        {
            handle_device_announce(&engine, &registry, &events_tx, &frame);
        }
    }
}

fn handle_device_announce(
    engine: &RequestEngine,
    registry: &Arc<Mutex<DeviceRegistry>>,
    events_tx: &Sender<ZigbeeEvent>,
    frame: &ZnpFrame,
) {
    if frame.data.len() < 12 {
        return;
    }

    let nwk_addr = u16::from_le_bytes([frame.data[2], frame.data[3]]);
    let mut ieee_address = [0u8; 8];
    ieee_address.copy_from_slice(&frame.data[4..12]);

    let base_device = Device {
        network_address: nwk_addr,
        ieee_address,
        endpoints: vec![],
    };

    if let Ok(mut reg) = registry.lock() {
        reg.register(base_device);
    }

    let endpoints = match query_endpoints(engine, nwk_addr) {
        Ok(eps) => eps,
        Err(_) => return,
    };

    let updated_device = Device {
        network_address: nwk_addr,
        ieee_address,
        endpoints,
    };

    if let Ok(mut reg) = registry.lock() {
        reg.register(updated_device.clone());
    }

    eprintln!(
        "[znp] device joined: addr=0x{:04X} ieee={:02X?} endpoints={:?}",
        updated_device.network_address,
        updated_device.ieee_address,
        updated_device
            .endpoints
            .iter()
            .map(|e| e.id)
            .collect::<Vec<_>>()
    );
    let _ = events_tx.send(ZigbeeEvent::DeviceJoined(updated_device));
}

fn query_endpoints(engine: &RequestEngine, nwk_addr: u16) -> Result<Vec<Endpoint>> {
    let addr_lo = (nwk_addr & 0xFF) as u8;
    let addr_hi = ((nwk_addr >> 8) & 0xFF) as u8;

    engine.sreq(
        subsystem::ZDO,
        subsystem::zdo::ACTIVE_EP_REQ,
        vec![addr_lo, addr_hi, addr_lo, addr_hi],
    )?;

    let rsp = engine.wait_for_areq(
        subsystem::ZDO,
        subsystem::zdo::ACTIVE_EP_RSP,
        EVENT_LOOP_TIMEOUT,
    )?;

    if rsp.data.len() < 6 {
        return Ok(vec![]);
    }

    let ep_count = rsp.data[5] as usize;
    if rsp.data.len() < 6 + ep_count {
        return Ok(vec![]);
    }

    let endpoint_ids: Vec<u8> = rsp.data[6..6 + ep_count].to_vec();

    let mut endpoints = Vec::with_capacity(endpoint_ids.len());
    for ep_id in endpoint_ids {
        if let Ok(ep) = query_simple_desc(engine, nwk_addr, ep_id) {
            endpoints.push(ep);
        }
    }

    Ok(endpoints)
}

fn query_simple_desc(engine: &RequestEngine, nwk_addr: u16, endpoint_id: u8) -> Result<Endpoint> {
    let addr_lo = (nwk_addr & 0xFF) as u8;
    let addr_hi = ((nwk_addr >> 8) & 0xFF) as u8;

    engine.sreq(
        subsystem::ZDO,
        subsystem::zdo::SIMPLE_DESC_REQ,
        vec![addr_lo, addr_hi, addr_lo, addr_hi, endpoint_id],
    )?;

    let rsp = engine.wait_for_areq(
        subsystem::ZDO,
        subsystem::zdo::SIMPLE_DESC_RSP,
        EVENT_LOOP_TIMEOUT,
    )?;

    if rsp.data.len() < 13 {
        return Ok(Endpoint {
            id: endpoint_id,
            input_clusters: vec![],
        });
    }

    let num_in_clusters = rsp.data[12] as usize;
    let clusters_start = 13;
    let clusters_end = clusters_start + num_in_clusters * 2;
    if rsp.data.len() < clusters_end {
        return Ok(Endpoint {
            id: endpoint_id,
            input_clusters: vec![],
        });
    }

    let input_clusters = (0..num_in_clusters)
        .map(|i| {
            let offset = clusters_start + i * 2;
            u16::from_le_bytes([rsp.data[offset], rsp.data[offset + 1]])
        })
        .collect();

    eprintln!(
        "[znp] endpoint {} clusters: {:?}",
        endpoint_id, input_clusters
    );

    Ok(Endpoint {
        id: endpoint_id,
        input_clusters,
    })
}

fn format_ieee(addr: &[u8; 8]) -> String {
    addr.iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(":")
}
