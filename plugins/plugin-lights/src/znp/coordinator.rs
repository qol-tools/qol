use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use anyhow::{ensure, Context, Result};

use super::request::RequestEngine;
use super::subsystem::*;
use super::zcl;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_TIMEOUT: Duration = Duration::from_millis(750);
const ENDPOINT_1: u8 = 0x01;
const PROFILE_HOME_AUTOMATION: u16 = 0x0104;
const DEVICE_CONFIGURATION_TOOL: u16 = 0x0005;
const AF_OPTIONS: u8 = 0x30;
const DEFAULT_RADIUS: u8 = 0x1E;
const STATUS_SUCCESS: u8 = 0x00;
const STATUS_AF_ALREADY_REGISTERED: u8 = 0x09;
const ADDR_MODE_SHORT: u8 = 0x02;
const BROADCAST_ROUTERS: u16 = 0xFFFC;
const COORDINATOR_STATE_STARTED: u8 = 0x09;
const DEVICE_VERSION: u8 = 0x00;
const LATENCY_NO_LATENCY: u8 = 0x00;
const DATA_CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);

static AF_TRANS_SEQ: AtomicU8 = AtomicU8::new(1);

pub struct NetworkConfig {
    pub pan_id: u16,
    pub channel: u8,
    pub network_key: [u8; 16],
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            pan_id: 0x1A62,
            channel: 11,
            network_key: [
                0x01, 0x03, 0x05, 0x07, 0x09, 0x0B, 0x0D, 0x0F, 0x00, 0x02, 0x04, 0x06, 0x08, 0x0A,
                0x0C, 0x0D,
            ],
        }
    }
}

pub struct NetworkInfo {
    pub ieee_address: [u8; 8],
    pub short_address: u16,
    pub pan_id: u16,
    pub channel: u8,
}

pub fn startup(engine: &RequestEngine, config: &NetworkConfig) -> Result<NetworkInfo> {
    eprintln!("[znp] resetting coordinator...");
    reset(engine).context("reset")?;
    eprintln!("[znp] ping...");
    ping(engine).context("ping")?;
    eprintln!(
        "[znp] configuring (channel={}, pan=0x{:04X})...",
        config.channel, config.pan_id
    );
    configure(engine, config).context("configure")?;
    eprintln!("[znp] registering endpoint...");
    register_endpoint(engine).context("register_endpoint")?;
    eprintln!("[znp] starting network...");
    start_network(engine).context("start_network")?;
    eprintln!("[znp] getting device info...");
    let info = get_device_info(engine).context("get_device_info")?;
    eprintln!("[znp] coordinator ready: addr=0x{:04X}", info.short_address);
    Ok(info)
}

pub fn probe(engine: &RequestEngine) -> Result<()> {
    engine
        .sreq_timeout(SYS, sys::PING, vec![], PROBE_TIMEOUT)
        .context("ping")?;
    Ok(())
}

pub fn get_device_info(engine: &RequestEngine) -> Result<NetworkInfo> {
    let resp = engine.sreq(UTIL, util::GET_DEVICE_INFO, vec![])?;
    ensure!(
        resp.data.len() >= 10,
        "GET_DEVICE_INFO response too short: {} bytes",
        resp.data.len()
    );

    let mut ieee_address = [0u8; 8];
    ieee_address.copy_from_slice(&resp.data[0..8]);
    let short_address = u16::from_le_bytes([resp.data[8], resp.data[9]]);

    Ok(NetworkInfo {
        ieee_address,
        short_address,
        pan_id: 0,
        channel: 0,
    })
}

pub fn permit_join(engine: &RequestEngine, duration_secs: u8) -> Result<()> {
    let addr_lo = (BROADCAST_ROUTERS & 0xFF) as u8;
    let addr_hi = ((BROADCAST_ROUTERS >> 8) & 0xFF) as u8;
    let data = vec![ADDR_MODE_SHORT, addr_lo, addr_hi, duration_secs, 0x00];
    let resp = engine.sreq(ZDO, zdo::MGMT_PERMIT_JOIN_REQ, data)?;
    ensure!(
        resp.data.first().copied() == Some(STATUS_SUCCESS),
        "MGMT_PERMIT_JOIN_REQ failed: status=0x{:02X}",
        resp.data.first().copied().unwrap_or(0xFF)
    );
    Ok(())
}

pub fn send_zcl_command(
    engine: &RequestEngine,
    dest_addr: u16,
    dest_endpoint: u8,
    cluster_id: u16,
    zcl_payload: &[u8],
) -> Result<()> {
    let dest_lo = (dest_addr & 0xFF) as u8;
    let dest_hi = ((dest_addr >> 8) & 0xFF) as u8;
    let cluster_lo = (cluster_id & 0xFF) as u8;
    let cluster_hi = ((cluster_id >> 8) & 0xFF) as u8;
    let trans_seq = next_af_trans_seq();

    let mut data = vec![
        dest_lo,
        dest_hi,
        dest_endpoint,
        ENDPOINT_1,
        cluster_lo,
        cluster_hi,
        trans_seq,
        AF_OPTIONS,
        DEFAULT_RADIUS,
        zcl_payload.len() as u8,
    ];
    data.extend_from_slice(zcl_payload);

    let resp = engine.sreq(AF, af::DATA_REQUEST, data)?;
    ensure!(
        resp.data.first().copied() == Some(STATUS_SUCCESS),
        "AF_DATA_REQUEST failed: status=0x{:02X}",
        resp.data.first().copied().unwrap_or(0xFF)
    );
    wait_for_data_confirm(engine, trans_seq)?;
    Ok(())
}

fn next_af_trans_seq() -> u8 {
    AF_TRANS_SEQ.fetch_add(1, Ordering::Relaxed)
}

fn wait_for_data_confirm(engine: &RequestEngine, trans_seq: u8) -> Result<()> {
    loop {
        let confirm = engine.wait_for_areq(AF, af::DATA_CONFIRM, DATA_CONFIRM_TIMEOUT)?;
        let Some(status) = confirm.data.first().copied() else {
            return Err(anyhow::anyhow!("AF_DATA_CONFIRM missing status byte"));
        };
        let Some(confirm_trans_seq) = confirm.data.get(2).copied() else {
            return Err(anyhow::anyhow!(
                "AF_DATA_CONFIRM missing transaction sequence"
            ));
        };
        if confirm_trans_seq != trans_seq {
            continue;
        }

        ensure!(
            status == STATUS_SUCCESS,
            "AF_DATA_CONFIRM failed: status=0x{:02X}",
            status
        );
        return Ok(());
    }
}

fn reset(engine: &RequestEngine) -> Result<()> {
    engine.areq(SYS, sys::RESET_REQ, vec![0x01])?;
    engine.wait_for_areq(SYS, sys::RESET_IND, STARTUP_TIMEOUT)?;
    Ok(())
}

fn ping(engine: &RequestEngine) -> Result<()> {
    engine.sreq(SYS, sys::PING, vec![])?;
    Ok(())
}

fn configure(engine: &RequestEngine, config: &NetworkConfig) -> Result<()> {
    nv_write(engine, nv_id::LOGICAL_TYPE, &[COORDINATOR])?;

    let pan_bytes = config.pan_id.to_le_bytes();
    nv_write(engine, nv_id::PAN_ID, &pan_bytes)?;

    let chanlist: u32 = 1u32 << config.channel;
    nv_write(engine, nv_id::CHANLIST, &chanlist.to_le_bytes())?;

    nv_write(engine, nv_id::PRECFGKEY, &config.network_key)?;
    nv_write(engine, nv_id::PRECFGKEYS_ENABLE, &[0x00])?;
    nv_write(engine, nv_id::ZDO_DIRECT_CB, &[0x01])?;

    Ok(())
}

fn nv_write(engine: &RequestEngine, item_id: u16, value: &[u8]) -> Result<()> {
    let id_bytes = item_id.to_le_bytes();
    let mut data = vec![id_bytes[0], id_bytes[1], 0x00, value.len() as u8];
    data.extend_from_slice(value);
    let resp = engine.sreq(SYS, sys::OSAL_NV_WRITE, data)?;
    ensure!(
        resp.data.first().copied() == Some(STATUS_SUCCESS),
        "SYS_OSAL_NV_WRITE failed for nv_id=0x{:04X}: status=0x{:02X}",
        item_id,
        resp.data.first().copied().unwrap_or(0xFF)
    );
    Ok(())
}

fn register_endpoint(engine: &RequestEngine) -> Result<()> {
    let profile_lo = (PROFILE_HOME_AUTOMATION & 0xFF) as u8;
    let profile_hi = ((PROFILE_HOME_AUTOMATION >> 8) & 0xFF) as u8;
    let device_lo = (DEVICE_CONFIGURATION_TOOL & 0xFF) as u8;
    let device_hi = ((DEVICE_CONFIGURATION_TOOL >> 8) & 0xFF) as u8;

    let input_clusters = [zcl::CLUSTER_ON_OFF, zcl::CLUSTER_LEVEL, zcl::CLUSTER_COLOR];

    let mut data = vec![
        ENDPOINT_1,
        profile_lo,
        profile_hi,
        device_lo,
        device_hi,
        DEVICE_VERSION,
        LATENCY_NO_LATENCY,
        input_clusters.len() as u8,
    ];
    for cluster in &input_clusters {
        data.push((cluster & 0xFF) as u8);
        data.push(((cluster >> 8) & 0xFF) as u8);
    }
    data.push(0x00);

    let resp = engine.sreq(AF, af::REGISTER, data)?;
    let status = resp.data.first().copied().unwrap_or(0xFF);
    ensure!(
        status == STATUS_SUCCESS || status == STATUS_AF_ALREADY_REGISTERED,
        "AF_REGISTER failed: status=0x{:02X}",
        status
    );
    Ok(())
}

fn start_network(engine: &RequestEngine) -> Result<()> {
    engine.sreq(ZDO, zdo::STARTUP_FROM_APP, vec![0x00, 0x00])?;
    let state_frame = engine.wait_for_areq(ZDO, zdo::STATE_CHANGE_IND, STARTUP_TIMEOUT)?;
    ensure!(
        state_frame.data.first().copied() == Some(COORDINATOR_STATE_STARTED),
        "unexpected coordinator state: 0x{:02X}",
        state_frame.data.first().copied().unwrap_or(0xFF)
    );
    Ok(())
}
