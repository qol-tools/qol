use std::path::Path;

use sha2::{Digest, Sha256};
use x11rb::connection::Connection as _;
use x11rb::protocol::randr::{self, ModeFlag};
use x11rb::protocol::xproto;
use x11rb::rust_connection::RustConnection;

use super::DisplayEnumerator;
use crate::display::{
    validate_layout, DisplayError, DisplayHandle, DisplayMode, DisplayOps, DisplayPlacement,
    DisplaySnapshot,
};
use crate::geometry::MonitorBounds;

const BASE_EDID_BYTES: usize = 128;

pub struct Platform;

impl DisplayEnumerator for Platform {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, DisplayError> {
        match enumerate_randr() {
            Ok(handles) if !handles.is_empty() => Ok(handles),
            _ => enumerate_from(Path::new("/sys/class/drm")),
        }
    }

    fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, DisplayError> {
        let mut bus = X11Bus::open()?;
        snapshot_from(&mut bus)
    }
}

impl DisplayOps for Platform {
    fn modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, DisplayError> {
        let mut bus = X11Bus::open()?;
        modes_from(&mut bus, handle)
    }

    fn set_mode(&self, handle: &DisplayHandle, mode: &DisplayMode) -> Result<(), DisplayError> {
        let mut bus = X11Bus::open()?;
        set_mode_from(&mut bus, handle, mode)
    }

    fn set_layout(&self, placements: &[DisplayPlacement]) -> Result<(), DisplayError> {
        let mut bus = X11Bus::open()?;
        set_layout_from(&mut bus, placements)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RandrOutput {
    output: u32,
    connector: String,
    connected: bool,
    crtc: u32,
    modes: Vec<u64>,
    edid: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RandrCrtc {
    crtc: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    mode: u64,
    outputs: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RandrMode {
    id: u64,
    width: u32,
    height: u32,
    dot_clock: u32,
    htotal: u32,
    vtotal: u32,
    interlace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RandrState {
    root: u32,
    config_timestamp: u32,
    primary: Option<u32>,
    outputs: Vec<RandrOutput>,
    crtcs: Vec<RandrCrtc>,
    modes: Vec<RandrMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RandrWrite {
    Applied,
    InvalidConfigTime,
}

trait RandrBus {
    fn read_state(&mut self) -> Result<RandrState, DisplayError>;

    fn set_crtc(
        &mut self,
        crtc: u32,
        x: i32,
        y: i32,
        mode: u64,
        outputs: &[u32],
        config_timestamp: u32,
    ) -> Result<RandrWrite, DisplayError>;

    fn set_primary(&mut self, root: u32, output: u32) -> Result<(), DisplayError>;
}

struct X11Bus {
    conn: RustConnection,
    screen: usize,
}

impl X11Bus {
    fn open() -> Result<Self, DisplayError> {
        let (conn, screen) = x11rb::connect(None).map_err(randr_failed)?;
        Ok(Self { conn, screen })
    }

    fn root(&self) -> Result<u32, DisplayError> {
        self.conn
            .setup()
            .roots
            .get(self.screen)
            .map(|root| root.root)
            .ok_or(DisplayError::UnsupportedPlatform)
    }
}

impl RandrBus for X11Bus {
    fn read_state(&mut self) -> Result<RandrState, DisplayError> {
        let root = self.root()?;
        let resources = randr::get_screen_resources_current(&self.conn, root)
            .map_err(randr_failed)?
            .reply()
            .map_err(randr_failed)?;
        let modes: Vec<RandrMode> = resources
            .modes
            .iter()
            .map(|mode| RandrMode {
                id: u64::from(mode.id),
                width: u32::from(mode.width),
                height: u32::from(mode.height),
                dot_clock: mode.dot_clock,
                htotal: u32::from(mode.htotal),
                vtotal: u32::from(mode.vtotal),
                interlace: mode.mode_flags.contains(ModeFlag::INTERLACE),
            })
            .collect();
        let primary = randr::get_output_primary(&self.conn, root)
            .map_err(randr_failed)?
            .reply()
            .map_err(randr_failed)?
            .output;
        let edid_atom = xproto::intern_atom(&self.conn, false, b"EDID")
            .map_err(randr_failed)?
            .reply()
            .map_err(randr_failed)?
            .atom;
        let mut outputs = Vec::with_capacity(resources.outputs.len());
        for output in resources.outputs {
            let info = randr::get_output_info(&self.conn, output, resources.config_timestamp)
                .map_err(randr_failed)?
                .reply()
                .map_err(randr_failed)?;
            let connected = info.connection == randr::Connection::CONNECTED;
            let edid = if connected && info.crtc != 0 {
                let property = randr::get_output_property(
                    &self.conn,
                    output,
                    edid_atom,
                    xproto::AtomEnum::ANY,
                    0,
                    32,
                    false,
                    false,
                )
                .map_err(randr_failed)?
                .reply()
                .map_err(randr_failed)?;
                (!property.data.is_empty()).then_some(property.data)
            } else {
                None
            };
            outputs.push(RandrOutput {
                output,
                connector: String::from_utf8_lossy(&info.name).into_owned(),
                connected,
                crtc: info.crtc,
                modes: info.modes.iter().map(|mode| u64::from(*mode)).collect(),
                edid,
            });
        }
        let mut crtcs = Vec::with_capacity(resources.crtcs.len());
        for crtc in resources.crtcs {
            let info = randr::get_crtc_info(&self.conn, crtc, resources.config_timestamp)
                .map_err(randr_failed)?
                .reply()
                .map_err(randr_failed)?;
            if info.status != randr::SetConfig::SUCCESS {
                continue;
            }
            crtcs.push(RandrCrtc {
                crtc,
                x: i32::from(info.x),
                y: i32::from(info.y),
                width: u32::from(info.width),
                height: u32::from(info.height),
                mode: u64::from(info.mode),
                outputs: info.outputs.clone(),
            });
        }
        Ok(RandrState {
            root,
            config_timestamp: resources.config_timestamp,
            primary: (primary != 0).then_some(primary),
            outputs,
            crtcs,
            modes,
        })
    }

    fn set_crtc(
        &mut self,
        crtc: u32,
        x: i32,
        y: i32,
        mode: u64,
        outputs: &[u32],
        config_timestamp: u32,
    ) -> Result<RandrWrite, DisplayError> {
        let reply = randr::set_crtc_config(
            &self.conn,
            crtc,
            x11rb::CURRENT_TIME,
            config_timestamp,
            x as i16,
            y as i16,
            mode as u32,
            randr::Rotation::ROTATE0,
            outputs,
        )
        .map_err(randr_failed)?
        .reply()
        .map_err(randr_failed)?;
        match reply.status {
            randr::SetConfig::SUCCESS => Ok(RandrWrite::Applied),
            randr::SetConfig::INVALID_CONFIG_TIME => Ok(RandrWrite::InvalidConfigTime),
            status => Err(DisplayError::Io(std::io::Error::other(format!(
                "the X11 server rejected the CRTC configuration: {status:?}"
            )))),
        }
    }

    fn set_primary(&mut self, root: u32, output: u32) -> Result<(), DisplayError> {
        randr::set_output_primary(&self.conn, root, output)
            .map_err(randr_failed)?
            .check()
            .map_err(randr_failed)
    }
}

fn refresh_hz(dot_clock: u32, htotal: u32, vtotal: u32, interlace: bool) -> u32 {
    let vtotal = if interlace {
        f64::from(vtotal) / 2.0
    } else {
        f64::from(vtotal)
    };
    let pixels = f64::from(htotal) * vtotal;
    if pixels <= 0.0 {
        return 0;
    }
    (f64::from(dot_clock) / pixels).round() as u32
}

fn mode_from_randr(mode: &RandrMode) -> DisplayMode {
    DisplayMode {
        token: mode.id,
        width: mode.width,
        height: mode.height,
        refresh_hz: refresh_hz(mode.dot_clock, mode.htotal, mode.vtotal, mode.interlace),
    }
}

fn handle_from_randr(connector: &str, edid: Option<&[u8]>) -> DisplayHandle {
    let (id, edid_sha256, identity_unstable) = identity_from(connector, edid);
    DisplayHandle::new(id, connector.to_string(), edid_sha256, identity_unstable)
}

fn output_for<'a>(state: &'a RandrState, handle: &DisplayHandle) -> Option<&'a RandrOutput> {
    state
        .outputs
        .iter()
        .find(|output| output.connector == handle.connector())
}

fn crtc_for<'a>(state: &'a RandrState, output: &RandrOutput) -> Option<&'a RandrCrtc> {
    if output.crtc == 0 {
        return None;
    }
    state.crtcs.iter().find(|crtc| crtc.crtc == output.crtc)
}

fn snapshot_from<B: RandrBus>(bus: &mut B) -> Result<Vec<DisplaySnapshot>, DisplayError> {
    let state = bus.read_state()?;
    let mut snapshots = Vec::new();
    for output in &state.outputs {
        if !output.connected || output.crtc == 0 {
            continue;
        }
        let Some(crtc) = state.crtcs.iter().find(|c| c.crtc == output.crtc) else {
            continue;
        };
        let mode = state
            .modes
            .iter()
            .find(|mode| mode.id == crtc.mode)
            .map(mode_from_randr);
        snapshots.push(DisplaySnapshot {
            handle: handle_from_randr(&output.connector, output.edid.as_deref()),
            bounds: MonitorBounds {
                x: crtc.x as f32,
                y: crtc.y as f32,
                width: crtc.width as f32,
                height: crtc.height as f32,
            },
            primary: state.primary == Some(output.output),
            mode,
        });
    }
    snapshots.sort_by(|a, b| a.handle.connector().cmp(b.handle.connector()));
    Ok(snapshots)
}

fn modes_from<B: RandrBus>(
    bus: &mut B,
    handle: &DisplayHandle,
) -> Result<Vec<DisplayMode>, DisplayError> {
    let state = bus.read_state()?;
    let output = output_for(&state, handle).ok_or_else(|| not_found("modes", handle))?;
    let mut ids = output.modes.clone();
    let crtc_mode = crtc_for(&state, output).map(|crtc| crtc.mode).unwrap_or(0);
    if crtc_mode != 0 && !ids.contains(&crtc_mode) {
        ids.push(crtc_mode);
    }
    let modes: Vec<DisplayMode> = ids
        .iter()
        .filter_map(|id| state.modes.iter().find(|mode| mode.id == *id))
        .map(mode_from_randr)
        .collect();
    if modes.is_empty() {
        return Err(DisplayError::Unsupported {
            capability: "modes",
            reason: format!(
                "no selectable modes are readable for {}",
                handle.connector()
            ),
        });
    }
    Ok(modes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CrtcWrite {
    crtc: u32,
    x: i32,
    y: i32,
    mode: u64,
    outputs: Vec<u32>,
}

fn not_found(capability: &'static str, handle: &DisplayHandle) -> DisplayError {
    DisplayError::NotFound {
        capability,
        selector: handle.connector().to_string(),
    }
}

fn set_mode_from<B: RandrBus>(
    bus: &mut B,
    handle: &DisplayHandle,
    mode: &DisplayMode,
) -> Result<(), DisplayError> {
    let pre = write_mode_with_retry(bus, handle, mode.token)?;
    let after = bus.read_state()?;
    let current = after
        .crtcs
        .iter()
        .find(|crtc| crtc.crtc == pre.crtc)
        .map(|crtc| crtc.mode);
    if current != Some(mode.token) {
        let observed = match current {
            Some(token) => token.to_string(),
            None => "none".to_string(),
        };
        let reason = match write_captured(bus, &pre) {
            Ok(()) => format!(
                "the mode write on {} did not verify: the server reports mode {}, requested {}",
                handle.connector(),
                observed,
                mode.token
            ),
            Err(error) => format!(
                "the mode write on {} did not verify: the server reports mode {}, requested {}; restoring the previous mode failed: {error}",
                handle.connector(),
                observed,
                mode.token
            ),
        };
        return Err(DisplayError::LayoutInvalid { reason });
    }
    Ok(())
}

fn write_mode_with_retry<B: RandrBus>(
    bus: &mut B,
    handle: &DisplayHandle,
    token: u64,
) -> Result<CrtcWrite, DisplayError> {
    let mut retried = false;
    loop {
        let state = bus.read_state()?;
        let output = output_for(&state, handle).ok_or_else(|| not_found("modes", handle))?;
        if !output.modes.contains(&token) {
            return Err(DisplayError::LayoutInvalid {
                reason: format!("mode {token} is not offered by {}", handle.connector()),
            });
        }
        let crtc = crtc_for(&state, output).ok_or_else(|| not_found("modes", handle))?;
        let pre = CrtcWrite {
            crtc: crtc.crtc,
            x: crtc.x,
            y: crtc.y,
            mode: crtc.mode,
            outputs: crtc.outputs.clone(),
        };
        match bus.set_crtc(
            crtc.crtc,
            crtc.x,
            crtc.y,
            token,
            &crtc.outputs,
            state.config_timestamp,
        )? {
            RandrWrite::Applied => return Ok(pre),
            RandrWrite::InvalidConfigTime if !retried => retried = true,
            RandrWrite::InvalidConfigTime => {
                return Err(DisplayError::Io(std::io::Error::other(
                    "the X11 server rejected the configuration with INVALID_CONFIG_TIME twice",
                )));
            }
        }
    }
}

fn write_placement_with_retry<B: RandrBus>(
    bus: &mut B,
    placement: &DisplayPlacement,
) -> Result<CrtcWrite, DisplayError> {
    let mut retried = false;
    loop {
        let state = bus.read_state()?;
        let Some(output) = output_for(&state, &placement.handle) else {
            return Err(not_found("layout", &placement.handle));
        };
        let Some(crtc) = crtc_for(&state, output) else {
            return Err(not_found("layout", &placement.handle));
        };
        let pre = CrtcWrite {
            crtc: crtc.crtc,
            x: crtc.x,
            y: crtc.y,
            mode: crtc.mode,
            outputs: crtc.outputs.clone(),
        };
        match bus.set_crtc(
            crtc.crtc,
            placement.x,
            placement.y,
            crtc.mode,
            &crtc.outputs,
            state.config_timestamp,
        )? {
            RandrWrite::Applied => return Ok(pre),
            RandrWrite::InvalidConfigTime if !retried => retried = true,
            RandrWrite::InvalidConfigTime => {
                return Err(DisplayError::Io(std::io::Error::other(format!(
                    "the X11 server rejected the configuration for {} with INVALID_CONFIG_TIME twice",
                    placement.handle.connector()
                ))));
            }
        }
    }
}

fn write_captured<B: RandrBus>(bus: &mut B, target: &CrtcWrite) -> Result<(), DisplayError> {
    let mut retried = false;
    loop {
        let config_timestamp = bus.read_state()?.config_timestamp;
        match bus.set_crtc(
            target.crtc,
            target.x,
            target.y,
            target.mode,
            &target.outputs,
            config_timestamp,
        )? {
            RandrWrite::Applied => return Ok(()),
            RandrWrite::InvalidConfigTime if !retried => retried = true,
            RandrWrite::InvalidConfigTime => {
                return Err(DisplayError::Io(std::io::Error::other(
                    "the X11 server rejected the rollback with INVALID_CONFIG_TIME twice",
                )));
            }
        }
    }
}

fn rollback_layout<B: RandrBus>(
    bus: &mut B,
    applied: &[CrtcWrite],
    error: DisplayError,
) -> DisplayError {
    let mut failures = Vec::new();
    for target in applied.iter().rev() {
        if let Err(failure) = write_captured(bus, target) {
            failures.push(failure.to_string());
        }
    }
    if failures.is_empty() {
        return error;
    }
    DisplayError::Io(std::io::Error::other(format!(
        "{error}; restoring the written displays failed: {}",
        failures.join("; ")
    )))
}

fn verify_placements<B: RandrBus>(
    bus: &mut B,
    placements: &[DisplayPlacement],
) -> Result<(), DisplayError> {
    let state = bus.read_state()?;
    for placement in placements {
        let Some(output) = output_for(&state, &placement.handle) else {
            return Err(not_found("layout", &placement.handle));
        };
        let Some(crtc) = crtc_for(&state, output) else {
            return Err(not_found("layout", &placement.handle));
        };
        if (crtc.x, crtc.y) != (placement.x, placement.y) {
            return Err(DisplayError::LayoutInvalid {
                reason: format!(
                    "the position for {} did not verify after the layout write",
                    placement.handle.connector()
                ),
            });
        }
        if placement.primary && state.primary != Some(output.output) {
            return Err(DisplayError::LayoutInvalid {
                reason: format!(
                    "the primary marker for {} did not verify after the layout write",
                    placement.handle.connector()
                ),
            });
        }
    }
    Ok(())
}

fn set_layout_from<B: RandrBus>(
    bus: &mut B,
    placements: &[DisplayPlacement],
) -> Result<(), DisplayError> {
    validate_layout(placements)?;
    for placement in placements {
        if i16::try_from(placement.x).is_err() || i16::try_from(placement.y).is_err() {
            return Err(DisplayError::LayoutInvalid {
                reason: format!(
                    "the position for {} is outside the X11 coordinate range",
                    placement.handle.connector()
                ),
            });
        }
    }
    let state = bus.read_state()?;
    let mut targets = Vec::with_capacity(placements.len());
    for placement in placements {
        let Some(output) = output_for(&state, &placement.handle) else {
            return Err(not_found("layout", &placement.handle));
        };
        let Some(crtc) = crtc_for(&state, output) else {
            return Err(not_found("layout", &placement.handle));
        };
        targets.push((placement, output, crtc));
    }
    for (index, (placement, _, crtc)) in targets.iter().enumerate() {
        for (other, _, other_crtc) in &targets[index + 1..] {
            let left = i64::from(placement.x);
            let top = i64::from(placement.y);
            let right = left + i64::from(crtc.width);
            let bottom = top + i64::from(crtc.height);
            let other_left = i64::from(other.x);
            let other_top = i64::from(other.y);
            let other_right = other_left + i64::from(other_crtc.width);
            let other_bottom = other_top + i64::from(other_crtc.height);
            let overlaps_x = left < other_right && other_left < right;
            let overlaps_y = top < other_bottom && other_top < bottom;
            if overlaps_x && overlaps_y {
                return Err(DisplayError::LayoutInvalid {
                    reason: format!(
                        "{} overlaps {}",
                        placement.handle.connector(),
                        other.handle.connector()
                    ),
                });
            }
        }
    }
    for (index, (_, _, crtc)) in targets.iter().enumerate() {
        if targets[..index]
            .iter()
            .any(|(_, _, prior)| prior.crtc == crtc.crtc)
        {
            return Err(DisplayError::LayoutInvalid {
                reason: "the layout mirrors two displays onto one CRTC".into(),
            });
        }
    }
    let mut applied: Vec<CrtcWrite> = Vec::new();
    for (placement, _, _) in &targets {
        match write_placement_with_retry(bus, placement) {
            Ok(pre) => applied.push(pre),
            Err(error) => return Err(rollback_layout(bus, &applied, error)),
        }
    }
    for (placement, output, _) in &targets {
        if !placement.primary {
            continue;
        }
        if let Err(error) = bus.set_primary(state.root, output.output) {
            return Err(rollback_layout(bus, &applied, error));
        }
    }
    verify_placements(bus, placements)?;
    Ok(())
}

fn enumerate_from(root: &Path) -> Result<Vec<DisplayHandle>, DisplayError> {
    let mut handles = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(connector) = connector_from_sys_name(&name) else {
            continue;
        };
        let connected = std::fs::read_to_string(entry.path().join("status"))
            .map(|status| status.trim() == "connected")
            .unwrap_or(false);
        if !connected {
            continue;
        }
        let (id, edid_sha256, identity_unstable) = identity_from(
            connector.as_str(),
            std::fs::read(entry.path().join("edid")).ok().as_deref(),
        );
        handles.push(DisplayHandle::new(
            id,
            connector,
            edid_sha256,
            identity_unstable,
        ));
    }
    handles.sort_by(|a, b| a.connector().cmp(b.connector()));
    Ok(handles)
}

fn identity_from(connector: &str, edid: Option<&[u8]>) -> (String, Option<[u8; 32]>, bool) {
    match edid {
        Some(base) => {
            let base = &base[..base.len().min(BASE_EDID_BYTES)];
            let digest: [u8; 32] = Sha256::digest(base).into();
            let mut hasher = Sha256::new();
            hasher.update(connector.as_bytes());
            hasher.update(base);
            let bound: [u8; 32] = hasher.finalize().into();
            (hex(&bound), Some(digest), false)
        }
        None => (
            hex(&Sha256::digest(connector.as_bytes()).into()),
            None,
            true,
        ),
    }
}

fn enumerate_randr() -> Result<Vec<DisplayHandle>, DisplayError> {
    let mut bus = X11Bus::open()?;
    let state = bus.read_state()?;
    let mut handles: Vec<DisplayHandle> = state
        .outputs
        .iter()
        .filter(|output| output.connected && output.crtc != 0)
        .map(|output| handle_from_randr(&output.connector, output.edid.as_deref()))
        .collect();
    handles.sort_by(|a, b| a.connector().cmp(b.connector()));
    Ok(handles)
}

fn randr_failed(error: impl std::fmt::Display) -> DisplayError {
    DisplayError::Io(std::io::Error::other(error.to_string()))
}

fn connector_from_sys_name(name: &str) -> Option<String> {
    let (card, connector) = name.split_once('-')?;
    if !card.starts_with("card") {
        return None;
    }
    (!connector.is_empty()).then(|| name.to_string())
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn identity_helper_matches_enumerate_from_construction() {
        let dir = tempdir().unwrap();
        let connector_dir = dir.path().join("card0-DP-1");
        fs::create_dir(&connector_dir).unwrap();
        fs::write(connector_dir.join("status"), "connected\n").unwrap();
        let edid = vec![0x7eu8; 200];
        fs::write(connector_dir.join("edid"), &edid).unwrap();

        let handles = enumerate_from(dir.path()).unwrap();
        let (id, edid_sha256, identity_unstable) = identity_from("card0-DP-1", Some(&edid));
        assert_eq!(handles[0].id(), id);
        assert_eq!(handles[0].edid_sha256(), edid_sha256);
        assert_eq!(handles[0].identity_unstable(), identity_unstable);
        assert!(!identity_unstable);
    }

    #[test]
    fn identity_helper_matches_enumerate_from_without_edid() {
        let dir = tempdir().unwrap();
        let connector_dir = dir.path().join("card0-DP-1");
        fs::create_dir(&connector_dir).unwrap();
        fs::write(connector_dir.join("status"), "connected\n").unwrap();

        let handles = enumerate_from(dir.path()).unwrap();
        let (id, edid_sha256, identity_unstable) = identity_from("card0-DP-1", None);
        assert_eq!(handles[0].id(), id);
        assert_eq!(handles[0].edid_sha256(), edid_sha256);
        assert_eq!(handles[0].identity_unstable(), identity_unstable);
        assert!(identity_unstable);
        assert_eq!(edid_sha256, None);
    }

    #[test]
    fn connector_from_sys_name_keeps_card_prefix() {
        assert_eq!(
            connector_from_sys_name("card0-DP-1").as_deref(),
            Some("card0-DP-1")
        );
        assert_eq!(
            connector_from_sys_name("card1-HDMI-A-1").as_deref(),
            Some("card1-HDMI-A-1")
        );
        assert_eq!(
            connector_from_sys_name("card0-eDP-1").as_deref(),
            Some("card0-eDP-1")
        );
        assert_eq!(connector_from_sys_name("card0"), None);
        assert_eq!(connector_from_sys_name("card0-"), None);
    }

    #[test]
    fn enumerates_connected_connectors_with_edid() {
        let dir = tempdir().unwrap();
        let connector_dir = dir.path().join("card0-DP-1");
        fs::create_dir(&connector_dir).unwrap();
        fs::write(connector_dir.join("status"), "connected\n").unwrap();
        let edid = vec![0x42; 128];
        fs::write(connector_dir.join("edid"), &edid).unwrap();

        let handles = enumerate_from(dir.path()).unwrap();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].connector(), "card0-DP-1");
        assert!(!handles[0].identity_unstable());
        let digest: [u8; 32] = Sha256::digest(&edid).into();
        assert_eq!(handles[0].edid_sha256(), Some(digest));
        let mut hasher = Sha256::new();
        hasher.update(b"card0-DP-1");
        hasher.update(&edid);
        let bound: [u8; 32] = hasher.finalize().into();
        assert_eq!(handles[0].id(), hex(&bound));
    }

    #[test]
    fn identical_edids_on_different_connectors_diverge() {
        let dir = tempdir().unwrap();
        for name in ["card0-DP-1", "card0-DP-2"] {
            let connector_dir = dir.path().join(name);
            fs::create_dir(&connector_dir).unwrap();
            fs::write(connector_dir.join("status"), "connected\n").unwrap();
            fs::write(connector_dir.join("edid"), [0x42; 128]).unwrap();
        }

        let handles = enumerate_from(dir.path()).unwrap();
        assert_eq!(handles.len(), 2);
        assert_eq!(handles[0].edid_sha256(), handles[1].edid_sha256());
        assert_ne!(handles[0].id(), handles[1].id());
    }

    #[test]
    fn identical_edids_on_different_cards_diverge() {
        let dir = tempdir().unwrap();
        for name in ["card0-DP-1", "card1-DP-1"] {
            let connector_dir = dir.path().join(name);
            fs::create_dir(&connector_dir).unwrap();
            fs::write(connector_dir.join("status"), "connected\n").unwrap();
            fs::write(connector_dir.join("edid"), [0x42; 128]).unwrap();
        }

        let handles = enumerate_from(dir.path()).unwrap();
        assert_eq!(handles.len(), 2);
        assert_eq!(handles[0].edid_sha256(), handles[1].edid_sha256());
        assert_ne!(handles[0].id(), handles[1].id());
    }

    #[test]
    fn identity_ignores_edid_extension_blocks() {
        let dir = tempdir().unwrap();
        let connector_dir = dir.path().join("card0-DP-1");
        fs::create_dir(&connector_dir).unwrap();
        fs::write(connector_dir.join("status"), "connected\n").unwrap();
        fs::write(connector_dir.join("edid"), [0x42; 256]).unwrap();

        let handles = enumerate_from(dir.path()).unwrap();
        let digest: [u8; 32] = Sha256::digest([0x42; 128]).into();
        assert_eq!(handles[0].edid_sha256(), Some(digest));
        let mut hasher = Sha256::new();
        hasher.update(b"card0-DP-1");
        hasher.update([0x42; 128]);
        assert_eq!(handles[0].id(), hex(&hasher.finalize().into()));
    }

    #[test]
    fn unreadable_edid_marks_identity_unstable() {
        let dir = tempdir().unwrap();
        let connector_dir = dir.path().join("card0-DP-1");
        fs::create_dir(&connector_dir).unwrap();
        fs::write(connector_dir.join("status"), "connected\n").unwrap();

        let handles = enumerate_from(dir.path()).unwrap();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].connector(), "card0-DP-1");
        assert!(handles[0].identity_unstable());
        assert_eq!(handles[0].edid_sha256(), None);
        let digest: [u8; 32] = Sha256::digest(b"card0-DP-1").into();
        assert_eq!(handles[0].id(), hex(&digest));
    }

    #[test]
    fn skips_disconnected_and_non_connector_entries() {
        let dir = tempdir().unwrap();
        let connected = dir.path().join("card0-HDMI-A-1");
        fs::create_dir(&connected).unwrap();
        fs::write(connected.join("status"), "connected\n").unwrap();
        fs::write(connected.join("edid"), [0u8; 128]).unwrap();
        let disconnected = dir.path().join("card0-DP-2");
        fs::create_dir(&disconnected).unwrap();
        fs::write(disconnected.join("status"), "disconnected\n").unwrap();
        fs::create_dir(dir.path().join("card0")).unwrap();
        fs::write(dir.path().join("card0").join("status"), "connected\n").unwrap();
        fs::write(dir.path().join("card0").join("edid"), [0u8; 128]).unwrap();

        let handles = enumerate_from(dir.path()).unwrap();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].connector(), "card0-HDMI-A-1");
    }

    #[test]
    fn sorts_by_connector_name() {
        let dir = tempdir().unwrap();
        for (card, connector) in [("card0-DP-2", "DP-2"), ("card0-DP-1", "DP-1")] {
            let connector_dir = dir.path().join(card);
            fs::create_dir(&connector_dir).unwrap();
            fs::write(connector_dir.join("status"), "connected\n").unwrap();
            fs::write(connector_dir.join("edid"), [0u8; 128]).unwrap();
            let _ = connector;
        }

        let handles = enumerate_from(dir.path()).unwrap();
        assert_eq!(
            handles.iter().map(|h| h.connector()).collect::<Vec<_>>(),
            vec!["card0-DP-1", "card0-DP-2"]
        );
    }

    fn fake_state() -> RandrState {
        RandrState {
            root: 7,
            config_timestamp: 100,
            primary: Some(100),
            outputs: vec![
                RandrOutput {
                    output: 100,
                    connector: "card0-DP-1".into(),
                    connected: true,
                    crtc: 200,
                    modes: vec![10, 11],
                    edid: Some(vec![0x11, 0x22]),
                },
                RandrOutput {
                    output: 101,
                    connector: "card0-DP-2".into(),
                    connected: true,
                    crtc: 201,
                    modes: vec![21],
                    edid: None,
                },
            ],
            crtcs: vec![
                RandrCrtc {
                    crtc: 200,
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                    mode: 10,
                    outputs: vec![100],
                },
                RandrCrtc {
                    crtc: 201,
                    x: 1920,
                    y: 0,
                    width: 2560,
                    height: 1440,
                    mode: 21,
                    outputs: vec![101],
                },
            ],
            modes: vec![
                RandrMode {
                    id: 10,
                    width: 1920,
                    height: 1080,
                    dot_clock: 148_500_000,
                    htotal: 2200,
                    vtotal: 1125,
                    interlace: false,
                },
                RandrMode {
                    id: 11,
                    width: 1920,
                    height: 1080,
                    dot_clock: 74_250_000,
                    htotal: 2200,
                    vtotal: 1125,
                    interlace: true,
                },
                RandrMode {
                    id: 21,
                    width: 2560,
                    height: 1440,
                    dot_clock: 241_500_000,
                    htotal: 2720,
                    vtotal: 1481,
                    interlace: false,
                },
            ],
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum FakeCall {
        Write(u32),
        Primary(u32),
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FakeWrite {
        crtc: u32,
        x: i32,
        y: i32,
        mode: u64,
        config_timestamp: u32,
    }

    struct FakeBus {
        state: RandrState,
        reads: usize,
        writes: Vec<FakeWrite>,
        calls: Vec<FakeCall>,
        fail_crtc: Option<u32>,
        fail_primary: bool,
        primary_failed: bool,
        fail_rollback: bool,
        invalid_config_time: Vec<(u32, u32)>,
        stale_on_invalid: Option<u32>,
        readback_override: Option<(u32, u64)>,
        readback_position_override: Option<(u32, i32, i32)>,
    }

    fn fake_bus(state: RandrState) -> FakeBus {
        FakeBus {
            state,
            reads: 0,
            writes: Vec::new(),
            calls: Vec::new(),
            fail_crtc: None,
            fail_primary: false,
            primary_failed: false,
            fail_rollback: false,
            invalid_config_time: Vec::new(),
            stale_on_invalid: None,
            readback_override: None,
            readback_position_override: None,
        }
    }

    impl RandrBus for FakeBus {
        fn read_state(&mut self) -> Result<RandrState, DisplayError> {
            self.reads += 1;
            self.state.config_timestamp += 1;
            Ok(self.state.clone())
        }

        fn set_crtc(
            &mut self,
            crtc: u32,
            x: i32,
            y: i32,
            mode: u64,
            _outputs: &[u32],
            config_timestamp: u32,
        ) -> Result<RandrWrite, DisplayError> {
            self.writes.push(FakeWrite {
                crtc,
                x,
                y,
                mode,
                config_timestamp,
            });
            self.calls.push(FakeCall::Write(crtc));
            if self.fail_crtc == Some(crtc) {
                return Err(DisplayError::Io(std::io::Error::other(
                    "the fake CRTC write failed",
                )));
            }
            if self.fail_rollback && self.primary_failed {
                return Err(DisplayError::Io(std::io::Error::other(
                    "the fake rollback write failed",
                )));
            }
            let mut invalid_answer = false;
            for (id, count) in self.invalid_config_time.iter_mut() {
                if *id == crtc && *count > 0 {
                    *count -= 1;
                    invalid_answer = true;
                    break;
                }
            }
            if invalid_answer && self.stale_on_invalid == Some(crtc) {
                self.state.crtcs.retain(|entry| entry.crtc != crtc);
                for output in self.state.outputs.iter_mut() {
                    if output.crtc == crtc {
                        output.crtc = 0;
                    }
                }
            }
            if invalid_answer {
                return Ok(RandrWrite::InvalidConfigTime);
            }
            if let Some(entry) = self.state.crtcs.iter_mut().find(|entry| entry.crtc == crtc) {
                entry.x = x;
                entry.y = y;
                entry.mode = mode;
            }
            if let Some((override_crtc, override_mode)) = self.readback_override {
                if let Some(entry) = self
                    .state
                    .crtcs
                    .iter_mut()
                    .find(|entry| entry.crtc == override_crtc)
                {
                    entry.mode = override_mode;
                }
            }
            let position_override = self.readback_position_override;
            if let Some((override_crtc, override_x, override_y)) = position_override {
                if let Some(entry) = self
                    .state
                    .crtcs
                    .iter_mut()
                    .find(|entry| entry.crtc == override_crtc)
                {
                    entry.x = override_x;
                    entry.y = override_y;
                }
            }
            Ok(RandrWrite::Applied)
        }

        fn set_primary(&mut self, _root: u32, output: u32) -> Result<(), DisplayError> {
            self.calls.push(FakeCall::Primary(output));
            if self.fail_primary {
                self.primary_failed = true;
                return Err(DisplayError::Io(std::io::Error::other(
                    "the fake primary write failed",
                )));
            }
            self.state.primary = Some(output);
            Ok(())
        }
    }

    fn fake_handle(connector: &str) -> DisplayHandle {
        handle_from_randr(connector, None)
    }

    fn fake_placement(connector: &str, x: i32, y: i32, primary: bool) -> DisplayPlacement {
        DisplayPlacement {
            handle: fake_handle(connector),
            x,
            y,
            primary,
        }
    }

    #[test]
    fn refresh_hz_rounds_a_59_94_style_mode() {
        assert_eq!(refresh_hz(27_000_000, 858, 525, false), 60);
        assert_eq!(refresh_hz(148_500_000, 2200, 1125, false), 60);
    }

    #[test]
    fn refresh_hz_accounts_for_interlace() {
        assert_eq!(refresh_hz(74_250_000, 2200, 1125, true), 60);
        assert_eq!(refresh_hz(74_250_000, 2200, 1125, false), 30);
    }

    #[test]
    fn refresh_hz_is_zero_without_a_vertical_total() {
        assert_eq!(refresh_hz(148_500_000, 2200, 0, false), 0);
    }

    #[test]
    fn snapshot_maps_position_mode_and_primary() {
        let mut bus = fake_bus(fake_state());
        let snapshots = snapshot_from(&mut bus).unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(
            snapshots[0].handle,
            handle_from_randr("card0-DP-1", Some(&[0x11, 0x22]))
        );
        assert_eq!(
            snapshots[0].bounds,
            MonitorBounds {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            }
        );
        assert!(snapshots[0].primary);
        assert_eq!(
            snapshots[0].mode,
            Some(DisplayMode {
                token: 10,
                width: 1920,
                height: 1080,
                refresh_hz: 60,
            })
        );
        assert_eq!(snapshots[1].handle, handle_from_randr("card0-DP-2", None));
        assert_eq!(
            snapshots[1].bounds,
            MonitorBounds {
                x: 1920.0,
                y: 0.0,
                width: 2560.0,
                height: 1440.0,
            }
        );
        assert!(!snapshots[1].primary);
        assert_eq!(snapshots[1].mode.as_ref().unwrap().token, 21);
    }

    #[test]
    fn snapshot_reports_no_mode_when_the_crtc_mode_is_unreadable() {
        let mut state = fake_state();
        state.crtcs[1].mode = 99;
        let mut bus = fake_bus(state);
        let snapshots = snapshot_from(&mut bus).unwrap();
        assert_eq!(snapshots[1].mode, None);
    }

    #[test]
    fn snapshot_skips_outputs_without_geometry() {
        let mut state = fake_state();
        state.outputs[1].crtc = 0;
        let mut bus = fake_bus(state);
        let snapshots = snapshot_from(&mut bus).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].handle.connector(), "card0-DP-1");
    }

    #[test]
    fn modes_list_every_selectable_mode_including_the_current_one() {
        let mut bus = fake_bus(fake_state());
        let handle = fake_handle("card0-DP-1");
        let modes = modes_from(&mut bus, &handle).unwrap();
        assert_eq!(
            modes.iter().map(|mode| mode.token).collect::<Vec<_>>(),
            vec![10, 11]
        );
        assert_eq!(modes[1].refresh_hz, 60);
    }

    #[test]
    fn modes_append_the_current_mode_when_the_output_omits_it() {
        let mut state = fake_state();
        state.outputs[0].modes = vec![11];
        let mut bus = fake_bus(state);
        let handle = fake_handle("card0-DP-1");
        let modes = modes_from(&mut bus, &handle).unwrap();
        assert_eq!(
            modes.iter().map(|mode| mode.token).collect::<Vec<_>>(),
            vec![11, 10]
        );
    }

    #[test]
    fn modes_refuse_a_display_without_a_readable_mode_list() {
        let mut state = fake_state();
        state.outputs[0].modes.clear();
        state.crtcs[0].mode = 0;
        let mut bus = fake_bus(state);
        let handle = fake_handle("card0-DP-1");
        assert!(matches!(
            modes_from(&mut bus, &handle),
            Err(DisplayError::Unsupported {
                capability: "modes",
                ..
            })
        ));
    }

    #[test]
    fn modes_refuse_an_unknown_connector() {
        let mut bus = fake_bus(fake_state());
        let handle = fake_handle("card0-HDMI-1");
        match modes_from(&mut bus, &handle) {
            Err(DisplayError::NotFound {
                capability,
                selector,
            }) => {
                assert_eq!(capability, "modes");
                assert_eq!(selector, "card0-HDMI-1");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn set_mode_writes_only_the_exact_token() {
        let mut bus = fake_bus(fake_state());
        let handle = fake_handle("card0-DP-1");
        let requested = DisplayMode {
            token: 11,
            width: 1,
            height: 1,
            refresh_hz: 1,
        };
        set_mode_from(&mut bus, &handle, &requested).unwrap();
        assert_eq!(bus.writes.len(), 1);
        assert_eq!(bus.writes[0].crtc, 200);
        assert_eq!(bus.writes[0].mode, 11);
    }

    #[test]
    fn set_mode_rolls_back_when_the_read_back_mismatches() {
        let mut bus = fake_bus(fake_state());
        bus.readback_override = Some((200, 10));
        let handle = fake_handle("card0-DP-1");
        let requested = DisplayMode {
            token: 11,
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };
        let error = set_mode_from(&mut bus, &handle, &requested).unwrap_err();
        match error {
            DisplayError::LayoutInvalid { reason } => {
                assert!(reason.contains("10"));
                assert!(reason.contains("11"));
            }
            other => panic!("expected LayoutInvalid, got {other:?}"),
        }
        assert_eq!(bus.writes.len(), 2);
        assert_eq!(
            (bus.writes[1].crtc, bus.writes[1].x, bus.writes[1].y),
            (200, 0, 0)
        );
        assert_eq!(bus.writes[1].mode, 10);
        let crtc = bus
            .state
            .crtcs
            .iter()
            .find(|crtc| crtc.crtc == 200)
            .unwrap();
        assert_eq!((crtc.x, crtc.y, crtc.mode), (0, 0, 10));
    }

    #[test]
    fn set_mode_refuses_a_token_the_target_output_does_not_offer() {
        let mut bus = fake_bus(fake_state());
        let handle = fake_handle("card0-DP-1");
        let requested = DisplayMode {
            token: 21,
            width: 2560,
            height: 1440,
            refresh_hz: 60,
        };
        assert!(matches!(
            set_mode_from(&mut bus, &handle, &requested),
            Err(DisplayError::LayoutInvalid { .. })
        ));
        assert!(bus.writes.is_empty());
    }

    #[test]
    fn set_mode_refuses_an_unknown_connector() {
        let mut bus = fake_bus(fake_state());
        let handle = fake_handle("card0-HDMI-1");
        let requested = DisplayMode {
            token: 10,
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };
        match set_mode_from(&mut bus, &handle, &requested) {
            Err(DisplayError::NotFound {
                capability,
                selector,
            }) => {
                assert_eq!(capability, "modes");
                assert_eq!(selector, "card0-HDMI-1");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
        assert!(bus.writes.is_empty());
    }

    #[test]
    fn set_layout_writes_every_crtc_then_the_primary_marker() {
        let mut bus = fake_bus(fake_state());
        let placements = vec![
            fake_placement("card0-DP-2", 0, 0, false),
            fake_placement("card0-DP-1", 2560, 0, true),
        ];
        set_layout_from(&mut bus, &placements).unwrap();
        assert_eq!(bus.writes.len(), 2);
        assert_eq!(bus.writes[0].crtc, 201);
        assert_eq!((bus.writes[0].x, bus.writes[0].y), (0, 0));
        assert_eq!(bus.writes[1].crtc, 200);
        assert_eq!((bus.writes[1].x, bus.writes[1].y), (2560, 0));
        assert_eq!(
            bus.calls,
            vec![
                FakeCall::Write(201),
                FakeCall::Write(200),
                FakeCall::Primary(100),
            ]
        );
    }

    #[test]
    fn set_layout_rolls_back_when_a_later_write_fails() {
        let mut bus = fake_bus(fake_state());
        bus.fail_crtc = Some(201);
        let placements = vec![
            fake_placement("card0-DP-1", 100, 0, true),
            fake_placement("card0-DP-2", 2200, 0, false),
        ];
        assert!(set_layout_from(&mut bus, &placements).is_err());
        let first = bus
            .state
            .crtcs
            .iter()
            .find(|crtc| crtc.crtc == 200)
            .unwrap();
        assert_eq!((first.x, first.y, first.mode), (0, 0, 10));
        let second = bus
            .state
            .crtcs
            .iter()
            .find(|crtc| crtc.crtc == 201)
            .unwrap();
        assert_eq!(second.x, 1920);
        assert_eq!(
            bus.writes.iter().filter(|write| write.crtc == 200).count(),
            2
        );
        assert!(bus
            .calls
            .iter()
            .all(|call| !matches!(call, FakeCall::Primary(_))));
    }

    #[test]
    fn set_layout_retries_once_on_invalid_config_time() {
        let mut bus = fake_bus(fake_state());
        bus.invalid_config_time = vec![(200, 1)];
        let placements = vec![fake_placement("card0-DP-1", 100, 0, true)];
        set_layout_from(&mut bus, &placements).unwrap();
        assert_eq!(bus.writes.len(), 2);
        assert!(bus.writes[1].config_timestamp > bus.writes[0].config_timestamp);
        assert_eq!(bus.writes[1].crtc, 200);
        assert_eq!(bus.writes[1].mode, 10);
    }

    #[test]
    fn set_layout_stops_after_two_invalid_config_time_answers() {
        let mut bus = fake_bus(fake_state());
        bus.invalid_config_time = vec![(201, 2)];
        let placements = vec![
            fake_placement("card0-DP-1", 100, 0, true),
            fake_placement("card0-DP-2", 2200, 0, false),
        ];
        assert!(matches!(
            set_layout_from(&mut bus, &placements),
            Err(DisplayError::Io(_))
        ));
        assert_eq!(
            bus.writes.iter().filter(|write| write.crtc == 200).count(),
            2
        );
        assert_eq!(
            bus.writes.iter().filter(|write| write.crtc == 201).count(),
            2
        );
        let first = bus
            .state
            .crtcs
            .iter()
            .find(|crtc| crtc.crtc == 200)
            .unwrap();
        assert_eq!((first.x, first.y, first.mode), (0, 0, 10));
        let second = bus
            .state
            .crtcs
            .iter()
            .find(|crtc| crtc.crtc == 201)
            .unwrap();
        assert_eq!((second.x, second.y), (1920, 0));
        assert!(bus
            .calls
            .iter()
            .all(|call| !matches!(call, FakeCall::Primary(_))));
    }

    #[test]
    fn set_layout_refuses_a_stale_target_after_a_config_change() {
        let mut bus = fake_bus(fake_state());
        bus.invalid_config_time = vec![(200, 1)];
        bus.stale_on_invalid = Some(200);
        let placements = vec![fake_placement("card0-DP-1", 100, 0, true)];
        match set_layout_from(&mut bus, &placements) {
            Err(DisplayError::NotFound {
                capability,
                selector,
            }) => {
                assert_eq!(capability, "layout");
                assert_eq!(selector, "card0-DP-1");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
        assert_eq!(bus.writes.len(), 1);
    }

    #[test]
    fn set_layout_refuses_when_the_position_does_not_read_back() {
        let mut bus = fake_bus(fake_state());
        bus.readback_position_override = Some((200, 500, 500));
        let placements = vec![fake_placement("card0-DP-1", 100, 0, true)];
        assert!(matches!(
            set_layout_from(&mut bus, &placements),
            Err(DisplayError::LayoutInvalid { .. })
        ));
        assert_eq!(bus.writes.len(), 1);
    }

    #[test]
    fn set_layout_rolls_back_when_the_primary_marker_fails() {
        let mut bus = fake_bus(fake_state());
        bus.fail_primary = true;
        let placements = vec![
            fake_placement("card0-DP-1", 100, 0, true),
            fake_placement("card0-DP-2", 2200, 0, false),
        ];
        let error = set_layout_from(&mut bus, &placements).unwrap_err();
        assert!(matches!(error, DisplayError::Io(_)));
        assert!(bus.calls.contains(&FakeCall::Primary(100)));
        let first = bus
            .state
            .crtcs
            .iter()
            .find(|crtc| crtc.crtc == 200)
            .unwrap();
        assert_eq!((first.x, first.y, first.mode), (0, 0, 10));
        let second = bus
            .state
            .crtcs
            .iter()
            .find(|crtc| crtc.crtc == 201)
            .unwrap();
        assert_eq!((second.x, second.y, second.mode), (1920, 0, 21));
    }

    #[test]
    fn set_layout_reports_a_failed_rollback_when_the_primary_marker_fails() {
        let mut bus = fake_bus(fake_state());
        bus.fail_primary = true;
        bus.fail_rollback = true;
        let placements = vec![
            fake_placement("card0-DP-1", 100, 0, true),
            fake_placement("card0-DP-2", 2200, 0, false),
        ];
        let error = set_layout_from(&mut bus, &placements).unwrap_err();
        match error {
            DisplayError::Io(error) => {
                let text = error.to_string();
                assert!(text.contains("restoring the written displays failed"));
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn set_layout_refuses_an_unknown_display() {
        let mut bus = fake_bus(fake_state());
        let placements = vec![fake_placement("card0-HDMI-1", 0, 0, true)];
        match set_layout_from(&mut bus, &placements) {
            Err(DisplayError::NotFound {
                capability,
                selector,
            }) => {
                assert_eq!(capability, "layout");
                assert_eq!(selector, "card0-HDMI-1");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
        assert!(bus.writes.is_empty());
    }

    #[test]
    fn set_layout_rejects_overlapping_rectangles() {
        let mut bus = fake_bus(fake_state());
        let placements = vec![
            fake_placement("card0-DP-1", 0, 0, true),
            fake_placement("card0-DP-2", 100, 100, false),
        ];
        assert!(matches!(
            set_layout_from(&mut bus, &placements),
            Err(DisplayError::LayoutInvalid { .. })
        ));
        assert!(bus.writes.is_empty());
    }

    #[test]
    fn set_layout_i16_boundary_table() {
        let cases = [
            (-32769, 0, false),
            (-32768, 0, true),
            (32767, 0, true),
            (32768, 0, false),
            (0, -32769, false),
            (0, -32768, true),
            (0, 32767, true),
            (0, 32768, false),
        ];
        for (x, y, accepted) in cases {
            let mut bus = fake_bus(fake_state());
            let placements = vec![fake_placement("card0-DP-1", x, y, true)];
            let result = set_layout_from(&mut bus, &placements);
            if accepted {
                assert!(result.is_ok(), "({x}, {y}) should be accepted");
            } else {
                assert!(
                    matches!(result, Err(DisplayError::LayoutInvalid { .. })),
                    "({x}, {y}) should be refused"
                );
                assert!(bus.writes.is_empty());
            }
        }
    }

    #[test]
    fn set_layout_rejects_mirrored_outputs() {
        let mut state = fake_state();
        state.outputs[1].crtc = 200;
        state.crtcs.remove(1);
        let mut bus = fake_bus(state);
        let placements = vec![
            fake_placement("card0-DP-1", 0, 0, true),
            fake_placement("card0-DP-2", 1920, 0, false),
        ];
        let error = set_layout_from(&mut bus, &placements).unwrap_err();
        match error {
            DisplayError::LayoutInvalid { reason } => assert!(reason.contains("mirror")),
            other => panic!("expected LayoutInvalid, got {other:?}"),
        }
        assert!(bus.writes.is_empty());
    }

    #[test]
    fn set_layout_refuses_a_missing_primary_before_touching_the_server() {
        let mut bus = fake_bus(fake_state());
        let placements = vec![
            fake_placement("card0-DP-1", 0, 0, false),
            fake_placement("card0-DP-2", 1920, 0, false),
        ];
        assert!(matches!(
            set_layout_from(&mut bus, &placements),
            Err(DisplayError::LayoutInvalid { .. })
        ));
        assert_eq!(bus.reads, 0);
    }
}
