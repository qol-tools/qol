use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

use super::{
    check_x11_range, crtc_for, exact_refresh, not_found, output_for, set_layout_from,
    set_mode_from, verify_placements, Persistence, RandrBus, RandrMode, RandrState,
};
use crate::display::{validate_layout, DisplayError, DisplayHandle, DisplayMode, DisplayPlacement};

const DESTINATION: &str = "org.cinnamon.Muffin.DisplayConfig";
const PATH: &str = "/org/cinnamon/Muffin/DisplayConfig";
const INTERFACE: &str = "org.cinnamon.Muffin.DisplayConfig";
const TEMPORARY_METHOD: u32 = 1;
const PERSISTENT_METHOD: u32 = 2;
const LOGICAL_LAYOUT_MODE: u32 = 1;
const REFRESH_TOLERANCE_HZ: f64 = 0.01;

type ModeBody = (
    String,
    i32,
    i32,
    f64,
    f64,
    Vec<f64>,
    HashMap<String, OwnedValue>,
);
type MonitorSpecBody = (String, String, String, String);
type MonitorBody = (MonitorSpecBody, Vec<ModeBody>, HashMap<String, OwnedValue>);
type LogicalMonitorBody = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<MonitorSpecBody>,
    HashMap<String, OwnedValue>,
);
type StateBody = (
    u32,
    Vec<MonitorBody>,
    Vec<LogicalMonitorBody>,
    HashMap<String, OwnedValue>,
);
type MonitorConfigBody<'a> = (&'a str, &'a str, HashMap<&'a str, Value<'a>>);
type LogicalMonitorConfigBody<'a> = (i32, i32, f64, u32, bool, Vec<MonitorConfigBody<'a>>);

#[derive(Debug, Clone, PartialEq)]
pub(super) struct MuffinMode {
    pub(super) id: String,
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) refresh: f64,
    pub(super) current: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct MuffinMonitor {
    pub(super) connector: String,
    pub(super) modes: Vec<MuffinMode>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct MuffinLogicalMonitor {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) scale: f64,
    pub(super) transform: u32,
    pub(super) primary: bool,
    pub(super) connectors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct MuffinState {
    pub(super) serial: u32,
    pub(super) monitors: Vec<MuffinMonitor>,
    pub(super) logical_monitors: Vec<MuffinLogicalMonitor>,
    pub(super) layout_mode: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct MonitorConfig {
    pub(super) connector: String,
    pub(super) mode_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LogicalMonitorConfig {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) scale: f64,
    pub(super) transform: u32,
    pub(super) primary: bool,
    pub(super) monitors: Vec<MonitorConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ModeSpec {
    width: u32,
    height: u32,
    interlace: bool,
    refresh: f64,
}

pub(super) trait MuffinBus {
    fn has_owner(&mut self) -> Result<bool, DisplayError>;

    fn current_state(&mut self) -> Result<MuffinState, DisplayError>;

    fn apply(
        &mut self,
        serial: u32,
        method: u32,
        logical_monitors: &[LogicalMonitorConfig],
    ) -> Result<(), DisplayError>;
}

pub(super) struct SessionMuffin {
    connection: zbus::blocking::Connection,
}

impl SessionMuffin {
    pub(super) fn connect() -> Option<Self> {
        zbus::blocking::Connection::session()
            .ok()
            .map(|connection| Self { connection })
    }
}

impl MuffinBus for SessionMuffin {
    fn has_owner(&mut self) -> Result<bool, DisplayError> {
        let reply = self
            .connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "NameHasOwner",
                &(DESTINATION,),
            )
            .map_err(muffin_failed)?;
        let owned: bool = reply.body().deserialize().map_err(muffin_failed)?;
        Ok(owned)
    }

    fn current_state(&mut self) -> Result<MuffinState, DisplayError> {
        let reply = self
            .connection
            .call_method(
                Some(DESTINATION),
                PATH,
                Some(INTERFACE),
                "GetCurrentState",
                &(),
            )
            .map_err(muffin_failed)?;
        let body: StateBody = reply.body().deserialize().map_err(muffin_failed)?;
        Ok(state_from_body(body))
    }

    fn apply(
        &mut self,
        serial: u32,
        method: u32,
        logical_monitors: &[LogicalMonitorConfig],
    ) -> Result<(), DisplayError> {
        let logical_monitors: Vec<LogicalMonitorConfigBody<'_>> =
            logical_monitors.iter().map(config_body).collect();
        let properties: HashMap<&str, Value<'_>> = HashMap::new();
        self.connection
            .call_method(
                Some(DESTINATION),
                PATH,
                Some(INTERFACE),
                "ApplyMonitorsConfig",
                &(serial, method, logical_monitors, properties),
            )
            .map_err(apply_failed)?;
        Ok(())
    }
}

pub(super) fn set_mode_with<M: MuffinBus, B: RandrBus>(
    muffin: &mut M,
    randr: &mut B,
    handle: &DisplayHandle,
    mode: &DisplayMode,
    persistence: Persistence,
) -> Result<(), DisplayError> {
    if !muffin.has_owner()? {
        return set_mode_from(randr, handle, mode);
    }
    let spec = requested_spec(&randr.read_state()?, handle, mode.token)?;
    let state = muffin.current_state()?;
    let logical_monitors = plan_mode(&state, handle.connector(), &spec)?;
    muffin.apply(state.serial, apply_method(persistence), &logical_monitors)?;
    verify_mode(randr, handle, &spec)
}

pub(super) fn set_layout_with<M: MuffinBus, B: RandrBus>(
    muffin: &mut M,
    randr: &mut B,
    placements: &[DisplayPlacement],
    persistence: Persistence,
) -> Result<(), DisplayError> {
    if !muffin.has_owner()? {
        return set_layout_from(randr, placements);
    }
    validate_layout(placements)?;
    check_x11_range(placements)?;
    let state = muffin.current_state()?;
    if state.layout_mode == Some(LOGICAL_LAYOUT_MODE) {
        return set_layout_from(randr, placements);
    }
    let logical_monitors = plan_layout(&state, placements)?;
    muffin.apply(state.serial, apply_method(persistence), &logical_monitors)?;
    verify_placements(randr, placements)
}

fn apply_method(persistence: Persistence) -> u32 {
    match persistence {
        Persistence::Session => TEMPORARY_METHOD,
        Persistence::Saved => PERSISTENT_METHOD,
    }
}

fn requested_spec(
    state: &RandrState,
    handle: &DisplayHandle,
    token: u64,
) -> Result<ModeSpec, DisplayError> {
    let output = output_for(state, handle).ok_or_else(|| not_found("modes", handle))?;
    let offered = state
        .modes
        .iter()
        .find(|mode| mode.id == token && output.modes.contains(&token));
    let Some(mode) = offered else {
        return Err(DisplayError::LayoutInvalid {
            reason: format!("mode {token} is not offered by {}", handle.connector()),
        });
    };
    Ok(spec_from_randr(mode))
}

fn verify_mode<B: RandrBus>(
    randr: &mut B,
    handle: &DisplayHandle,
    spec: &ModeSpec,
) -> Result<(), DisplayError> {
    let state = randr.read_state()?;
    let output = output_for(&state, handle).ok_or_else(|| not_found("modes", handle))?;
    let current = crtc_for(&state, output)
        .and_then(|crtc| state.modes.iter().find(|mode| mode.id == crtc.mode))
        .map(spec_from_randr);
    if current.is_some_and(|current| matches_spec(&current, spec)) {
        return Ok(());
    }
    let observed = current.map_or_else(|| "none".to_string(), |current| describe(&current));
    Err(DisplayError::LayoutInvalid {
        reason: format!(
            "the mode write on {} did not verify: the server reports mode {observed}, requested {}",
            handle.connector(),
            describe(spec)
        ),
    })
}

fn spec_from_randr(mode: &RandrMode) -> ModeSpec {
    ModeSpec {
        width: mode.width,
        height: mode.height,
        interlace: mode.interlace,
        refresh: exact_refresh(mode.dot_clock, mode.htotal, mode.vtotal, mode.interlace),
    }
}

fn matches_spec(current: &ModeSpec, spec: &ModeSpec) -> bool {
    current.width == spec.width
        && current.height == spec.height
        && current.interlace == spec.interlace
        && (current.refresh - spec.refresh).abs() <= REFRESH_TOLERANCE_HZ
}

fn describe(spec: &ModeSpec) -> String {
    let scan = if spec.interlace { "i" } else { "" };
    format!("{}x{}{scan}@{:.3}", spec.width, spec.height, spec.refresh)
}

fn interlaced(id: &str) -> bool {
    id.split('@').next().is_some_and(|size| size.ends_with('i'))
}

fn same_shape(mode: &MuffinMode, spec: &ModeSpec) -> bool {
    i64::from(mode.width) == i64::from(spec.width)
        && i64::from(mode.height) == i64::from(spec.height)
        && interlaced(&mode.id) == spec.interlace
}

fn mode_for<'a>(
    monitor: &'a MuffinMonitor,
    spec: &ModeSpec,
) -> Result<&'a MuffinMode, DisplayError> {
    monitor
        .modes
        .iter()
        .filter(|mode| same_shape(mode, spec))
        .min_by(|a, b| {
            (a.refresh - spec.refresh)
                .abs()
                .total_cmp(&(b.refresh - spec.refresh).abs())
        })
        .filter(|mode| (mode.refresh - spec.refresh).abs() <= REFRESH_TOLERANCE_HZ)
        .ok_or_else(|| DisplayError::LayoutInvalid {
            reason: format!(
                "Muffin offers no mode within {REFRESH_TOLERANCE_HZ} Hz of {} on {}",
                describe(spec),
                monitor.connector
            ),
        })
}

fn connector_not_found(capability: &'static str, connector: &str) -> DisplayError {
    DisplayError::NotFound {
        capability,
        selector: connector.to_string(),
    }
}

fn current_mode<'a>(
    state: &'a MuffinState,
    connector: &str,
) -> Result<&'a MuffinMode, DisplayError> {
    state
        .monitors
        .iter()
        .find(|monitor| monitor.connector == connector)
        .and_then(|monitor| monitor.modes.iter().find(|mode| mode.current))
        .ok_or_else(|| {
            DisplayError::Io(std::io::Error::other(format!(
                "Muffin reports no current mode for {connector}"
            )))
        })
}

fn current_monitor_config(
    state: &MuffinState,
    connector: &str,
) -> Result<MonitorConfig, DisplayError> {
    let mode = current_mode(state, connector)?;
    Ok(MonitorConfig {
        connector: connector.to_string(),
        mode_id: mode.id.clone(),
    })
}

fn logical_config(
    state: &MuffinState,
    logical: &MuffinLogicalMonitor,
) -> Result<LogicalMonitorConfig, DisplayError> {
    let monitors = logical
        .connectors
        .iter()
        .map(|connector| current_monitor_config(state, connector))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LogicalMonitorConfig {
        x: logical.x,
        y: logical.y,
        scale: logical.scale,
        transform: logical.transform,
        primary: logical.primary,
        monitors,
    })
}

fn current_configs(state: &MuffinState) -> Result<Vec<LogicalMonitorConfig>, DisplayError> {
    state
        .logical_monitors
        .iter()
        .map(|logical| logical_config(state, logical))
        .collect()
}

fn plan_mode(
    state: &MuffinState,
    connector: &str,
    spec: &ModeSpec,
) -> Result<Vec<LogicalMonitorConfig>, DisplayError> {
    let monitor = state
        .monitors
        .iter()
        .find(|monitor| monitor.connector == connector)
        .ok_or_else(|| connector_not_found("modes", connector))?;
    let new_mode = mode_for(monitor, spec)?;
    let mut configs = current_configs(state)?;
    let index = state
        .logical_monitors
        .iter()
        .position(|logical| logical.connectors.iter().any(|name| name == connector))
        .ok_or_else(|| connector_not_found("modes", connector))?;
    let logical = &state.logical_monitors[index];
    let (old_width, old_height) =
        logical_extent(current_mode(state, connector)?, logical, state.layout_mode);
    let (new_width, new_height) = logical_extent(new_mode, logical, state.layout_mode);
    let mut extents = Vec::with_capacity(configs.len());
    for other in &state.logical_monitors {
        extents.push(match other.connectors.first() {
            Some(first) => logical_extent(current_mode(state, first)?, other, state.layout_mode),
            None => (0, 0),
        });
    }
    let columns: Vec<Span> = state
        .logical_monitors
        .iter()
        .zip(&extents)
        .map(|(other, &(width, height))| Span {
            start: other.x,
            length: width,
            cross_start: other.y,
            cross_length: height,
        })
        .collect();
    let rows: Vec<Span> = state
        .logical_monitors
        .iter()
        .zip(&extents)
        .map(|(other, &(width, height))| Span {
            start: other.y,
            length: height,
            cross_start: other.x,
            cross_length: width,
        })
        .collect();
    let shift_x = trailing_neighbours(&columns, index);
    let shift_y = trailing_neighbours(&rows, index);
    for (other, config) in configs.iter_mut().enumerate() {
        if other == index {
            if let Some(target) = config
                .monitors
                .iter_mut()
                .find(|monitor| monitor.connector == connector)
            {
                target.mode_id = new_mode.id.clone();
            }
            continue;
        }
        if shift_x[other] {
            config.x += new_width - old_width;
        }
        if shift_y[other] {
            config.y += new_height - old_height;
        }
    }
    Ok(configs)
}

/// A logical monitor's extent along one axis plus its extent on the perpendicular axis.
struct Span {
    start: i32,
    length: i32,
    cross_start: i32,
    cross_length: i32,
}

/// Flags the monitors that sit past the far edge of `changed` along the axis, directly or
/// through a chain of monitors that do, and share rows (or columns) with the one before them.
fn trailing_neighbours(spans: &[Span], changed: usize) -> Vec<bool> {
    let mut shifted = vec![false; spans.len()];
    shifted[changed] = true;
    let mut grew = true;
    while grew {
        grew = false;
        for other in 0..spans.len() {
            if shifted[other] {
                continue;
            }
            let follows = (0..spans.len()).any(|anchor| {
                let (a, o) = (&spans[anchor], &spans[other]);
                shifted[anchor]
                    && o.start >= a.start + a.length
                    && o.cross_start < a.cross_start + a.cross_length
                    && a.cross_start < o.cross_start + o.cross_length
            });
            if follows {
                shifted[other] = true;
                grew = true;
            }
        }
    }
    shifted[changed] = false;
    shifted
}

fn logical_extent(
    mode: &MuffinMode,
    logical: &MuffinLogicalMonitor,
    layout_mode: Option<u32>,
) -> (i32, i32) {
    let (width, height) = if logical.transform % 2 == 1 {
        (mode.height, mode.width)
    } else {
        (mode.width, mode.height)
    };
    if layout_mode != Some(LOGICAL_LAYOUT_MODE) {
        return (width, height);
    }
    let scaled = |length: i32| (f64::from(length) / logical.scale).round() as i32;
    (scaled(width), scaled(height))
}

fn logical_position(
    logical: &MuffinLogicalMonitor,
    placements: &[DisplayPlacement],
) -> Result<(i32, i32), DisplayError> {
    let mut positions = logical.connectors.iter().map(|connector| {
        placements
            .iter()
            .find(|placement| placement.handle.connector() == connector)
            .map_or((logical.x, logical.y), |placement| {
                (placement.x, placement.y)
            })
    });
    let Some(first) = positions.next() else {
        return Ok((logical.x, logical.y));
    };
    if positions.any(|position| position != first) {
        return Err(DisplayError::LayoutInvalid {
            reason: format!(
                "the layout splits the mirrored displays {}",
                logical.connectors.join(", ")
            ),
        });
    }
    Ok(first)
}

fn plan_layout(
    state: &MuffinState,
    placements: &[DisplayPlacement],
) -> Result<Vec<LogicalMonitorConfig>, DisplayError> {
    let mut primary = None;
    for placement in placements {
        let index = state
            .logical_monitors
            .iter()
            .position(|logical| {
                logical
                    .connectors
                    .iter()
                    .any(|connector| connector == placement.handle.connector())
            })
            .ok_or_else(|| not_found("layout", &placement.handle))?;
        if placement.primary {
            primary = Some(index);
        }
    }
    let mut configs = current_configs(state)?;
    for (index, (config, logical)) in configs.iter_mut().zip(&state.logical_monitors).enumerate() {
        (config.x, config.y) = logical_position(logical, placements)?;
        if let Some(primary) = primary {
            config.primary = index == primary;
        }
    }
    Ok(configs)
}

fn config_body(config: &LogicalMonitorConfig) -> LogicalMonitorConfigBody<'_> {
    let monitors = config
        .monitors
        .iter()
        .map(|monitor| {
            (
                monitor.connector.as_str(),
                monitor.mode_id.as_str(),
                HashMap::new(),
            )
        })
        .collect();
    (
        config.x,
        config.y,
        config.scale,
        config.transform,
        config.primary,
        monitors,
    )
}

fn mode_from_body((id, width, height, refresh, _, _, properties): ModeBody) -> MuffinMode {
    let current = properties
        .get("is-current")
        .and_then(|value| bool::try_from(value).ok())
        .unwrap_or(false);
    MuffinMode {
        id,
        width,
        height,
        refresh,
        current,
    }
}

fn monitor_from_body(((connector, _, _, _), modes, _): MonitorBody) -> MuffinMonitor {
    MuffinMonitor {
        connector,
        modes: modes.into_iter().map(mode_from_body).collect(),
    }
}

fn logical_monitor_from_body(
    (x, y, scale, transform, primary, monitors, _): LogicalMonitorBody,
) -> MuffinLogicalMonitor {
    MuffinLogicalMonitor {
        x,
        y,
        scale,
        transform,
        primary,
        connectors: monitors
            .into_iter()
            .map(|(connector, _, _, _)| connector)
            .collect(),
    }
}

fn state_from_body((serial, monitors, logical_monitors, properties): StateBody) -> MuffinState {
    MuffinState {
        serial,
        monitors: monitors.into_iter().map(monitor_from_body).collect(),
        logical_monitors: logical_monitors
            .into_iter()
            .map(logical_monitor_from_body)
            .collect(),
        layout_mode: properties
            .get("layout-mode")
            .and_then(|value| u32::try_from(value).ok()),
    }
}

fn muffin_failed(error: impl std::fmt::Display) -> DisplayError {
    DisplayError::Io(std::io::Error::other(error.to_string()))
}

fn apply_failed(error: impl std::fmt::Display) -> DisplayError {
    DisplayError::Io(std::io::Error::other(format!(
        "Muffin refused the monitor configuration: {error}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(id: &str, width: i32, height: i32, refresh: f64, current: bool) -> MuffinMode {
        MuffinMode {
            id: id.into(),
            width,
            height,
            refresh,
            current,
        }
    }

    fn logical(x: i32, y: i32, primary: bool, connectors: &[&str]) -> MuffinLogicalMonitor {
        MuffinLogicalMonitor {
            x,
            y,
            scale: 1.0,
            transform: 0,
            primary,
            connectors: connectors
                .iter()
                .map(|connector| connector.to_string())
                .collect(),
        }
    }

    fn state() -> MuffinState {
        MuffinState {
            serial: 9,
            monitors: vec![
                MuffinMonitor {
                    connector: "DP-1".into(),
                    modes: vec![
                        mode("1920x1080@60.000", 1920, 1080, 60.0, true),
                        mode("1920x1080@59.940", 1920, 1080, 59.940_06, false),
                        mode("1920x1080i@60.000", 1920, 1080, 60.0, false),
                    ],
                },
                MuffinMonitor {
                    connector: "DP-2".into(),
                    modes: vec![
                        mode("2560x1440@143.981", 2560, 1440, 143.981, false),
                        mode("2560x1440@59.951", 2560, 1440, 59.951, true),
                    ],
                },
                MuffinMonitor {
                    connector: "HDMI-1".into(),
                    modes: vec![mode("1920x1080@60.000", 1920, 1080, 60.0, true)],
                },
            ],
            logical_monitors: vec![
                logical(0, 0, true, &["DP-1"]),
                logical(1920, 0, false, &["DP-2"]),
                logical(4480, 0, false, &["HDMI-1"]),
            ],
            layout_mode: Some(2),
        }
    }

    fn mirrored_state() -> MuffinState {
        let mut state = state();
        state.logical_monitors = vec![
            logical(0, 0, true, &["DP-1", "HDMI-1"]),
            logical(1920, 0, false, &["DP-2"]),
        ];
        state
    }

    fn spec(width: u32, height: u32, interlace: bool, refresh: f64) -> ModeSpec {
        ModeSpec {
            width,
            height,
            interlace,
            refresh,
        }
    }

    fn placement(connector: &str, x: i32, y: i32, primary: bool) -> DisplayPlacement {
        DisplayPlacement {
            handle: DisplayHandle::new(connector.into(), connector.into(), None, false),
            x,
            y,
            primary,
        }
    }

    fn config(x: i32, y: i32, primary: bool, monitors: &[(&str, &str)]) -> LogicalMonitorConfig {
        LogicalMonitorConfig {
            x,
            y,
            scale: 1.0,
            transform: 0,
            primary,
            monitors: monitors
                .iter()
                .map(|(connector, mode_id)| MonitorConfig {
                    connector: connector.to_string(),
                    mode_id: mode_id.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn interlace_is_read_from_the_mode_id() {
        assert!(interlaced("1920x1080i@60.000"));
        assert!(!interlaced("1920x1080@143.981"));
    }

    #[test]
    fn mode_id_tells_59_94_from_60() {
        let state = state();
        let monitor = &state.monitors[0];
        assert_eq!(
            mode_for(monitor, &spec(1920, 1080, false, 59.940_06))
                .unwrap()
                .id,
            "1920x1080@59.940"
        );
        assert_eq!(
            mode_for(monitor, &spec(1920, 1080, false, 60.0))
                .unwrap()
                .id,
            "1920x1080@60.000"
        );
    }

    #[test]
    fn mode_id_keeps_interlaced_and_progressive_apart() {
        let state = state();
        let monitor = &state.monitors[0];
        assert_eq!(
            mode_for(monitor, &spec(1920, 1080, true, 60.0)).unwrap().id,
            "1920x1080i@60.000"
        );
    }

    #[test]
    fn mode_id_picks_the_closest_refresh_within_the_tolerance() {
        let state = state();
        let monitor = &state.monitors[0];
        assert_eq!(
            mode_for(monitor, &spec(1920, 1080, false, 59.945))
                .unwrap()
                .id,
            "1920x1080@59.940"
        );
        assert_eq!(
            mode_for(monitor, &spec(1920, 1080, false, 60.009))
                .unwrap()
                .id,
            "1920x1080@60.000"
        );
    }

    #[test]
    fn mode_id_refuses_a_refresh_or_size_muffin_does_not_offer() {
        let state = state();
        let monitor = &state.monitors[0];
        for wanted in [
            spec(1920, 1080, false, 59.5),
            spec(1920, 1080, false, 60.02),
            spec(2560, 1440, false, 60.0),
            spec(1920, 1080, true, 59.94),
        ] {
            assert!(matches!(
                mode_for(monitor, &wanted),
                Err(DisplayError::LayoutInvalid { .. })
            ));
        }
    }

    #[test]
    fn plan_mode_swaps_only_the_target_mode_id() {
        let planned = plan_mode(&state(), "DP-2", &spec(2560, 1440, false, 143.98)).unwrap();
        assert_eq!(
            planned,
            vec![
                config(0, 0, true, &[("DP-1", "1920x1080@60.000")]),
                config(1920, 0, false, &[("DP-2", "2560x1440@143.981")]),
                config(4480, 0, false, &[("HDMI-1", "1920x1080@60.000")]),
            ]
        );
    }

    #[test]
    fn plan_mode_keeps_the_other_monitor_of_a_mirror() {
        let planned =
            plan_mode(&mirrored_state(), "DP-1", &spec(1920, 1080, false, 59.94)).unwrap();
        assert_eq!(
            planned,
            vec![
                config(
                    0,
                    0,
                    true,
                    &[("DP-1", "1920x1080@59.940"), ("HDMI-1", "1920x1080@60.000")]
                ),
                config(1920, 0, false, &[("DP-2", "2560x1440@59.951")]),
            ]
        );
    }

    fn with_small_dp1(current: bool) -> MuffinState {
        let mut state = state();
        state.monitors[0].modes[0].current = !current;
        state.monitors[0]
            .modes
            .push(mode("1280x720@60.000", 1280, 720, 60.0, current));
        state
    }

    fn positions(planned: &[LogicalMonitorConfig]) -> Vec<(i32, i32)> {
        planned.iter().map(|config| (config.x, config.y)).collect()
    }

    #[test]
    fn plan_mode_leaves_positions_alone_on_a_refresh_change() {
        let planned = plan_mode(&state(), "DP-1", &spec(1920, 1080, false, 59.94)).unwrap();
        assert_eq!(positions(&planned), vec![(0, 0), (1920, 0), (4480, 0)]);
    }

    #[test]
    fn plan_mode_pulls_right_neighbours_left_when_the_left_monitor_shrinks() {
        let planned = plan_mode(
            &with_small_dp1(false),
            "DP-1",
            &spec(1280, 720, false, 60.0),
        )
        .unwrap();
        assert_eq!(planned[0].monitors[0].mode_id, "1280x720@60.000");
        assert_eq!(positions(&planned), vec![(0, 0), (1280, 0), (3840, 0)]);
    }

    #[test]
    fn plan_mode_pushes_lower_neighbours_down_when_the_top_monitor_grows() {
        let mut state = with_small_dp1(true);
        state.logical_monitors = vec![
            logical(0, 0, true, &["DP-1"]),
            logical(0, 720, false, &["DP-2"]),
            logical(1280, 0, false, &["HDMI-1"]),
        ];
        let planned = plan_mode(&state, "DP-1", &spec(1920, 1080, false, 60.0)).unwrap();
        assert_eq!(positions(&planned), vec![(0, 0), (0, 1080), (1920, 0)]);
    }

    #[test]
    fn plan_mode_leaves_monitors_in_other_rows_and_columns_alone() {
        let mut state = with_small_dp1(false);
        state.logical_monitors = vec![
            logical(0, 0, true, &["DP-2"]),
            logical(0, 1440, false, &["DP-1"]),
            logical(2560, 0, false, &["HDMI-1"]),
        ];
        let planned = plan_mode(&state, "DP-1", &spec(1280, 720, false, 60.0)).unwrap();
        assert_eq!(positions(&planned), vec![(0, 0), (0, 1440), (2560, 0)]);
    }

    #[test]
    fn plan_mode_swaps_width_and_height_for_a_rotated_monitor() {
        let mut state = with_small_dp1(false);
        state.logical_monitors = vec![
            logical(0, 0, true, &["DP-1"]),
            logical(1080, 0, false, &["DP-2"]),
            logical(3640, 0, false, &["HDMI-1"]),
        ];
        state.logical_monitors[0].transform = 1;
        let planned = plan_mode(&state, "DP-1", &spec(1280, 720, false, 60.0)).unwrap();
        assert_eq!(positions(&planned), vec![(0, 0), (720, 0), (3280, 0)]);
    }

    #[test]
    fn plan_mode_divides_by_the_scale_in_logical_layout_mode() {
        let mut state = with_small_dp1(false);
        state.layout_mode = Some(LOGICAL_LAYOUT_MODE);
        state.logical_monitors = vec![
            logical(0, 0, true, &["DP-1"]),
            logical(960, 0, false, &["DP-2"]),
            logical(3520, 0, false, &["HDMI-1"]),
        ];
        state.logical_monitors[0].scale = 2.0;
        let planned = plan_mode(&state, "DP-1", &spec(1280, 720, false, 60.0)).unwrap();
        assert_eq!(positions(&planned), vec![(0, 0), (640, 0), (3200, 0)]);
    }

    #[test]
    fn plan_mode_refuses_an_unknown_or_disabled_connector() {
        let mut state = state();
        state.logical_monitors.pop();
        for connector in ["DP-9", "HDMI-1"] {
            assert!(matches!(
                plan_mode(&state, connector, &spec(1920, 1080, false, 60.0)),
                Err(DisplayError::NotFound {
                    capability: "modes",
                    ..
                })
            ));
        }
    }

    #[test]
    fn plan_mode_refuses_a_monitor_without_a_current_mode() {
        let mut state = state();
        state.monitors[2].modes[0].current = false;
        assert!(matches!(
            plan_mode(&state, "DP-1", &spec(1920, 1080, false, 59.94)),
            Err(DisplayError::Io(_))
        ));
    }

    #[test]
    fn plan_layout_moves_positions_and_primary() {
        let planned = plan_layout(
            &state(),
            &[
                placement("DP-1", 2560, 0, false),
                placement("DP-2", 0, 0, true),
            ],
        )
        .unwrap();
        assert_eq!(
            planned,
            vec![
                config(2560, 0, false, &[("DP-1", "1920x1080@60.000")]),
                config(0, 0, true, &[("DP-2", "2560x1440@59.951")]),
                config(4480, 0, false, &[("HDMI-1", "1920x1080@60.000")]),
            ]
        );
    }

    #[test]
    fn plan_layout_moves_a_mirror_as_one() {
        let planned = plan_layout(
            &mirrored_state(),
            &[
                placement("DP-1", 0, 1440, true),
                placement("HDMI-1", 0, 1440, false),
                placement("DP-2", 0, 0, false),
            ],
        )
        .unwrap();
        assert_eq!((planned[0].x, planned[0].y), (0, 1440));
        assert!(planned[0].primary);
        assert_eq!((planned[1].x, planned[1].y), (0, 0));
    }

    #[test]
    fn plan_layout_refuses_splitting_a_mirror() {
        for placements in [
            vec![
                placement("DP-1", 0, 0, true),
                placement("HDMI-1", 1920, 0, false),
            ],
            vec![placement("DP-1", 0, 1440, true)],
        ] {
            assert!(matches!(
                plan_layout(&mirrored_state(), &placements),
                Err(DisplayError::LayoutInvalid { .. })
            ));
        }
    }

    #[test]
    fn plan_layout_refuses_an_unknown_connector() {
        assert!(matches!(
            plan_layout(&state(), &[placement("DP-9", 0, 0, true)]),
            Err(DisplayError::NotFound {
                capability: "layout",
                ..
            })
        ));
    }

    #[test]
    fn persistence_picks_the_apply_method() {
        assert_eq!(apply_method(Persistence::Session), 1);
        assert_eq!(apply_method(Persistence::Saved), 2);
    }

    #[test]
    fn apply_failures_carry_the_muffin_message() {
        let error =
            apply_failed("org.freedesktop.DBus.Error.InvalidArgs: Logical monitors not adjacent");
        assert!(matches!(error, DisplayError::Io(_)));
        assert!(error.to_string().contains("Logical monitors not adjacent"));
    }

    #[test]
    fn state_is_parsed_from_the_get_current_state_body() {
        let mode_properties = HashMap::from([("is-current".to_string(), OwnedValue::from(true))]);
        let monitor: MonitorBody = (
            ("DP-1".into(), "DEL".into(), "U2720Q".into(), "ABC".into()),
            vec![
                (
                    "1920x1080@60.000".into(),
                    1920,
                    1080,
                    60.0,
                    1.0,
                    vec![1.0],
                    mode_properties,
                ),
                (
                    "1920x1080@59.940".into(),
                    1920,
                    1080,
                    59.94,
                    1.0,
                    vec![1.0],
                    HashMap::new(),
                ),
            ],
            HashMap::new(),
        );
        let logical: LogicalMonitorBody = (
            10,
            20,
            2.0,
            1,
            true,
            vec![("DP-1".into(), "DEL".into(), "U2720Q".into(), "ABC".into())],
            HashMap::new(),
        );
        let properties = HashMap::from([("layout-mode".to_string(), OwnedValue::from(2u32))]);
        let parsed = state_from_body((5, vec![monitor], vec![logical], properties));
        assert_eq!(
            parsed,
            MuffinState {
                serial: 5,
                monitors: vec![MuffinMonitor {
                    connector: "DP-1".into(),
                    modes: vec![
                        mode("1920x1080@60.000", 1920, 1080, 60.0, true),
                        mode("1920x1080@59.940", 1920, 1080, 59.94, false),
                    ],
                }],
                logical_monitors: vec![MuffinLogicalMonitor {
                    x: 10,
                    y: 20,
                    scale: 2.0,
                    transform: 1,
                    primary: true,
                    connectors: vec!["DP-1".into()],
                }],
                layout_mode: Some(2),
            }
        );
    }
}
