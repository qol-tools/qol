use std::collections::BTreeMap;

use qol_windowing::display::DisplayHandle;

use crate::monitor::{DisplayMode, DisplayPlacement, DisplaySnapshot, MonitorError};

#[derive(serde::Serialize)]
pub struct LayoutRow {
    pub id: String,
    pub connector: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub primary: bool,
    pub detail: String,
    pub settable: bool,
}

#[derive(serde::Serialize)]
pub struct ModeRow {
    pub id: String,
    pub display_id: String,
    pub connector: String,
    pub token: u64,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub label: String,
    pub detail: String,
    pub current: bool,
    pub writable: bool,
    pub selectable: bool,
}

#[derive(Debug, serde::Deserialize, Clone)]
pub struct ArrangeRequest {
    pub id: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq)]
pub struct LayoutPosition {
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub primary: bool,
}

pub(crate) fn origin_x(snapshot: &DisplaySnapshot) -> i32 {
    snapshot.bounds.x.round() as i32
}

pub(crate) fn origin_y(snapshot: &DisplaySnapshot) -> i32 {
    snapshot.bounds.y.round() as i32
}

fn width_of(snapshot: &DisplaySnapshot) -> u32 {
    snapshot.bounds.width.round() as u32
}

fn height_of(snapshot: &DisplaySnapshot) -> u32 {
    snapshot.bounds.height.round() as u32
}

pub fn geometry_detail(snapshot: &DisplaySnapshot) -> String {
    let mut detail = format!(
        "{:+}{:+} {}x{}",
        origin_x(snapshot),
        origin_y(snapshot),
        width_of(snapshot),
        height_of(snapshot),
    );
    if let Some(mode) = &snapshot.mode {
        detail.push_str(&format!("@{}", mode.refresh_hz));
    }
    if snapshot.primary {
        detail.push_str(" primary");
    }
    detail
}

pub fn placements_from_snapshots(snapshots: &[DisplaySnapshot]) -> Vec<DisplayPlacement> {
    snapshots
        .iter()
        .map(|snapshot| DisplayPlacement {
            handle: snapshot.handle.clone(),
            x: origin_x(snapshot),
            y: origin_y(snapshot),
            primary: snapshot.primary,
        })
        .collect()
}

pub fn layout_rows(snapshots: &[DisplaySnapshot]) -> Vec<LayoutRow> {
    snapshots
        .iter()
        .map(|snapshot| LayoutRow {
            id: snapshot.handle.id().to_string(),
            connector: snapshot.handle.connector().to_string(),
            x: origin_x(snapshot),
            y: origin_y(snapshot),
            width: width_of(snapshot),
            height: height_of(snapshot),
            refresh_hz: snapshot.mode.as_ref().map_or(0, |mode| mode.refresh_hz),
            primary: snapshot.primary,
            detail: geometry_detail(snapshot),
            settable: !snapshot.primary,
        })
        .collect()
}

pub fn mode_rows(
    snapshots: &[DisplaySnapshot],
    modes: &BTreeMap<String, Vec<DisplayMode>>,
    writable: bool,
) -> Vec<ModeRow> {
    let mut rows = Vec::new();
    for snapshot in snapshots {
        let Some(list) = modes
            .get(snapshot.handle.connector())
            .or_else(|| modes.get(snapshot.handle.id()))
        else {
            continue;
        };
        let current_token = snapshot.mode.as_ref().map(|mode| mode.token);
        for mode in list {
            let current = current_token == Some(mode.token);
            let detail = if current {
                "current mode"
            } else {
                "available mode"
            };
            let display_id = snapshot.handle.id().to_string();
            rows.push(ModeRow {
                id: format!("{display_id}#{}", mode.token),
                display_id,
                connector: snapshot.handle.connector().to_string(),
                token: mode.token,
                width: mode.width,
                height: mode.height,
                refresh_hz: mode.refresh_hz,
                label: format!("{}x{}@{}", mode.width, mode.height, mode.refresh_hz),
                detail: detail.to_string(),
                current,
                writable,
                selectable: writable && !current,
            });
        }
    }
    rows
}

pub fn mode_lists<F>(
    snapshots: &[DisplaySnapshot],
    mut list: F,
) -> BTreeMap<String, Vec<DisplayMode>>
where
    F: FnMut(&DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError>,
{
    let mut modes = BTreeMap::new();
    for snapshot in snapshots {
        if let Ok(entries) = list(&snapshot.handle) {
            modes.insert(snapshot.handle.connector().to_string(), entries);
        }
    }
    modes
}

fn mode_candidates(modes: &[&DisplayMode]) -> String {
    modes
        .iter()
        .map(|mode| format!("{}x{}@{}", mode.width, mode.height, mode.refresh_hz))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn resolve_mode(
    modes: &[DisplayMode],
    width: u32,
    height: u32,
    refresh: Option<u32>,
) -> Result<DisplayMode, MonitorError> {
    let candidates: Vec<&DisplayMode> = modes
        .iter()
        .filter(|mode| mode.width == width && mode.height == height)
        .filter(|mode| refresh.is_none_or(|refresh| mode.refresh_hz == refresh))
        .collect();
    let requested = match refresh {
        Some(refresh) => format!("{width}x{height}@{refresh}"),
        None => format!("{width}x{height}"),
    };
    match candidates.as_slice() {
        [] => {
            let available = if modes.is_empty() {
                "none".to_string()
            } else {
                mode_candidates(&modes.iter().collect::<Vec<_>>())
            };
            Err(MonitorError::refused(
                "modes",
                format!("no mode matches {requested}; available: {available}"),
            ))
        }
        [mode] => Ok((*mode).clone()),
        many if refresh.is_none() => Err(MonitorError::refused(
            "modes",
            format!(
                "resolution is ambiguous without a refresh rate, candidates: {}",
                mode_candidates(many)
            ),
        )),
        many => Err(MonitorError::refused(
            "modes",
            format!(
                "several modes match {requested}, candidates: {}",
                mode_candidates(many)
            ),
        )),
    }
}

pub fn snapshot_for<'a>(
    snapshots: &'a [DisplaySnapshot],
    selector: &str,
) -> Result<&'a DisplaySnapshot, MonitorError> {
    match_index(snapshots, selector).map(|index| &snapshots[index])
}

fn match_index(snapshots: &[DisplaySnapshot], selector: &str) -> Result<usize, MonitorError> {
    let ids: Vec<usize> = snapshots
        .iter()
        .enumerate()
        .filter(|(_, snapshot)| snapshot.handle.id() == selector)
        .map(|(index, _)| index)
        .collect();
    match ids.as_slice() {
        [index] => return Ok(*index),
        [] => {}
        many => return Err(ambiguous_selector(snapshots, selector, many)),
    }
    let connectors: Vec<usize> = snapshots
        .iter()
        .enumerate()
        .filter(|(_, snapshot)| snapshot.handle.connector() == selector)
        .map(|(index, _)| index)
        .collect();
    match connectors.as_slice() {
        [index] => Ok(*index),
        [] => Err(MonitorError::DisplayNotFound(selector.to_string())),
        many => Err(ambiguous_selector(snapshots, selector, many)),
    }
}

fn ambiguous_selector(
    snapshots: &[DisplaySnapshot],
    selector: &str,
    matches: &[usize],
) -> MonitorError {
    let connectors = matches
        .iter()
        .map(|index| snapshots[*index].handle.connector().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    MonitorError::refused(
        "display",
        format!("display `{selector}` is ambiguous; candidates: {connectors}"),
    )
}

fn layout_index(snapshots: &[DisplaySnapshot], selector: &str) -> Result<usize, MonitorError> {
    match match_index(snapshots, selector) {
        Err(MonitorError::DisplayNotFound(_)) => Err(unknown_display(selector)),
        other => other,
    }
}

fn unknown_display(selector: &str) -> MonitorError {
    MonitorError::refused("layout", format!("unknown display `{selector}`"))
}

fn current_primary(snapshots: &[DisplaySnapshot]) -> Result<usize, MonitorError> {
    let mut primaries = snapshots
        .iter()
        .enumerate()
        .filter(|(_, snapshot)| snapshot.primary)
        .map(|(index, _)| index);
    let Some(first) = primaries.next() else {
        return Err(MonitorError::refused(
            "layout",
            "no connected display is marked primary",
        ));
    };
    if primaries.next().is_some() {
        return Err(MonitorError::refused(
            "layout",
            "several connected displays are marked primary",
        ));
    }
    Ok(first)
}

fn mark_primary(placements: &mut [DisplayPlacement], primary: usize) {
    for (index, placement) in placements.iter_mut().enumerate() {
        placement.primary = index == primary;
    }
}

pub fn resolve_arrange(
    snapshots: &[DisplaySnapshot],
    requested: &[ArrangeRequest],
    primary: Option<&str>,
) -> Result<Vec<DisplayPlacement>, MonitorError> {
    if snapshots.is_empty() {
        return Err(MonitorError::refused("layout", "no connected displays"));
    }
    let mut placements = placements_from_snapshots(snapshots);
    let mut requested_indexes = BTreeMap::<usize, String>::new();
    for request in requested {
        let index = layout_index(snapshots, &request.id)?;
        if requested_indexes
            .insert(index, request.id.clone())
            .is_some()
        {
            return Err(MonitorError::refused(
                "layout",
                format!("display `{}` is requested more than once", request.id),
            ));
        }
        placements[index].x = request.x;
        placements[index].y = request.y;
    }
    let primary = match primary {
        Some(selector) => layout_index(snapshots, selector)?,
        None => current_primary(snapshots)?,
    };
    mark_primary(&mut placements, primary);
    Ok(placements)
}

pub fn resolve_config_layout(
    snapshots: &[DisplaySnapshot],
    config: &BTreeMap<String, LayoutPosition>,
) -> Result<Vec<DisplayPlacement>, MonitorError> {
    if snapshots.is_empty() {
        return Err(MonitorError::refused("layout", "no connected displays"));
    }
    let mut placements = placements_from_snapshots(snapshots);
    let mut configured_primary = None;
    let mut configured_indexes = BTreeMap::<usize, String>::new();
    for (selector, position) in config {
        let index = layout_index(snapshots, selector)?;
        if configured_indexes.insert(index, selector.clone()).is_some() {
            return Err(MonitorError::refused(
                "layout",
                format!("display `{selector}` is configured more than once"),
            ));
        }
        placements[index].x = position.x;
        placements[index].y = position.y;
        if position.primary {
            if configured_primary.is_some() {
                return Err(MonitorError::refused(
                    "layout",
                    "several configured displays are marked primary",
                ));
            }
            configured_primary = Some(index);
        }
    }
    let primary = match configured_primary {
        Some(index) => index,
        None => current_primary(snapshots)?,
    };
    mark_primary(&mut placements, primary);
    Ok(placements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_windowing::MonitorBounds;

    fn handle(id: &str, connector: &str) -> DisplayHandle {
        DisplayHandle::new(id.to_string(), connector.to_string(), None, false)
    }

    fn snapshot(
        id: &str,
        connector: &str,
        x: f32,
        y: f32,
        primary: bool,
        refresh_hz: Option<u32>,
    ) -> DisplaySnapshot {
        snapshot_with_mode(
            id,
            connector,
            x,
            y,
            primary,
            refresh_hz.map(|refresh_hz| DisplayMode {
                token: u64::from(refresh_hz),
                width: 1920,
                height: 1080,
                refresh_hz,
            }),
        )
    }

    fn snapshot_with_mode(
        id: &str,
        connector: &str,
        x: f32,
        y: f32,
        primary: bool,
        mode: Option<DisplayMode>,
    ) -> DisplaySnapshot {
        DisplaySnapshot {
            handle: handle(id, connector),
            bounds: MonitorBounds {
                x,
                y,
                width: 1920.0,
                height: 1080.0,
            },
            primary,
            mode,
        }
    }

    fn mode(token: u64, width: u32, height: u32, refresh_hz: u32) -> DisplayMode {
        DisplayMode {
            token,
            width,
            height,
            refresh_hz,
        }
    }

    fn refused(result: Result<Vec<DisplayPlacement>, MonitorError>) -> String {
        match result {
            Err(MonitorError::Refused { capability, reason }) => {
                assert_eq!(capability, "layout");
                reason
            }
            other => panic!("expected a layout refusal, got {other:?}"),
        }
    }

    fn refused_reason(result: Result<DisplayMode, MonitorError>) -> String {
        match result {
            Err(MonitorError::Refused { reason, .. }) => reason,
            other => panic!("expected a mode refusal, got {other:?}"),
        }
    }

    #[test]
    fn layout_rows_carry_position_size_mode_and_primary() {
        let snapshots = vec![
            snapshot("left", "card0-DP-1", -1920.0, 0.0, false, Some(60)),
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(144)),
        ];
        let rows = layout_rows(&snapshots);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "left");
        assert_eq!(rows[0].connector, "card0-DP-1");
        assert_eq!((rows[0].x, rows[0].y), (-1920, 0));
        assert_eq!((rows[0].width, rows[0].height), (1920, 1080));
        assert_eq!(rows[0].refresh_hz, 60);
        assert_eq!(rows[0].detail, "-1920+0 1920x1080@60");
        assert!(!rows[0].primary);
        assert!(rows[0].settable);
        assert_eq!(rows[1].detail, "+0+0 1920x1080@144 primary");
        assert!(rows[1].primary);
        assert!(!rows[1].settable);
    }

    #[test]
    fn layout_rows_omit_refresh_when_the_current_mode_is_unreadable() {
        let rows = layout_rows(&[snapshot("only", "card0-DP-1", 0.0, 0.0, true, None)]);
        assert_eq!(rows[0].refresh_hz, 0);
        assert_eq!(rows[0].detail, "+0+0 1920x1080 primary");
    }

    #[test]
    fn placements_from_snapshots_keep_handle_geometry_and_primary() {
        let snapshots = vec![
            snapshot("left", "card0-DP-1", -1920.0, 0.0, false, Some(60)),
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60)),
        ];
        let placements = placements_from_snapshots(&snapshots);
        assert_eq!(placements.len(), 2);
        assert_eq!(
            placements[0],
            DisplayPlacement {
                handle: handle("left", "card0-DP-1"),
                x: -1920,
                y: 0,
                primary: false,
            }
        );
        assert_eq!(placements[1].handle, handle("main", "card0-HDMI-1"));
        assert!(placements[1].primary);
    }

    #[test]
    fn mode_rows_mark_the_current_mode_and_hide_its_action() {
        let snapshots = vec![snapshot_with_mode(
            "main",
            "card0-HDMI-1",
            0.0,
            0.0,
            true,
            Some(mode(11, 1920, 1080, 60)),
        )];
        let modes = BTreeMap::from([(
            "card0-HDMI-1".to_string(),
            vec![mode(11, 1920, 1080, 60), mode(12, 2560, 1440, 144)],
        )]);
        let rows = mode_rows(&snapshots, &modes, true);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "main#11");
        assert_eq!(rows[0].display_id, "main");
        assert_eq!(rows[0].connector, "card0-HDMI-1");
        assert_eq!(rows[0].token, 11);
        assert_eq!(rows[0].label, "1920x1080@60");
        assert_eq!(rows[0].detail, "current mode");
        assert!(rows[0].current);
        assert!(rows[0].writable);
        assert!(!rows[0].selectable);
        assert_eq!(rows[1].id, "main#12");
        assert_eq!(rows[1].token, 12);
        assert_eq!(rows[1].label, "2560x1440@144");
        assert_eq!(rows[1].detail, "available mode");
        assert!(!rows[1].current);
        assert!(rows[1].selectable);
    }

    #[test]
    fn mode_rows_address_each_row_by_display_and_token() {
        let snapshots = vec![snapshot_with_mode(
            "main",
            "card0-HDMI-1",
            0.0,
            0.0,
            true,
            Some(mode(10, 1920, 1080, 60)),
        )];
        let modes = BTreeMap::from([(
            "card0-HDMI-1".to_string(),
            vec![
                mode(10, 1920, 1080, 60),
                mode(11, 1920, 1080, 60),
                mode(12, 2560, 1440, 144),
            ],
        )]);
        let rows = mode_rows(&snapshots, &modes, true);
        assert_eq!(rows.len(), 3);
        let ids = rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids, ["main#10", "main#11", "main#12"]);
        assert!(!rows[0].selectable);
        assert!(rows[1].selectable);
        assert!(rows[2].selectable);
        assert_eq!(
            rows[0].label, rows[1].label,
            "colliding labels stay distinct rows"
        );
        assert_ne!(rows[0].id, rows[1].id);
    }

    #[test]
    fn mode_rows_match_the_map_by_connector_or_display_id() {
        let snapshots = vec![
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60)),
            snapshot("left", "card0-DP-1", -1920.0, 0.0, false, Some(60)),
        ];
        let by_connector =
            BTreeMap::from([("card0-HDMI-1".to_string(), vec![mode(21, 1280, 720, 75)])]);
        let rows = mode_rows(&snapshots, &by_connector, true);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display_id, "main");
        assert_eq!(rows[0].id, "main#21");
        assert_eq!((rows[0].width, rows[0].height), (1280, 720));
        assert_eq!(rows[0].refresh_hz, 75);
        assert!(!rows[0].current);
        assert!(rows[0].selectable);
        let by_id = BTreeMap::from([("left".to_string(), vec![mode(22, 1280, 720, 75)])]);
        let rows = mode_rows(&snapshots, &by_id, true);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display_id, "left");
        assert_eq!(rows[0].id, "left#22");
    }

    #[test]
    fn mode_rows_leave_every_row_unmarked_when_the_mode_is_unreadable() {
        let snapshots = vec![snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, None)];
        let modes = BTreeMap::from([("main".to_string(), vec![mode(31, 1920, 1080, 60)])]);
        let rows = mode_rows(&snapshots, &modes, true);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].current);
        assert!(rows[0].selectable);
    }

    #[test]
    fn mode_rows_disable_every_action_when_the_backend_cannot_write_modes() {
        let snapshots = vec![snapshot_with_mode(
            "main",
            "card0-HDMI-1",
            0.0,
            0.0,
            true,
            Some(mode(41, 1920, 1080, 60)),
        )];
        let modes = BTreeMap::from([(
            "card0-HDMI-1".to_string(),
            vec![mode(41, 1920, 1080, 60), mode(42, 2560, 1440, 144)],
        )]);
        let rows = mode_rows(&snapshots, &modes, false);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| !row.writable));
        assert!(rows.iter().all(|row| !row.selectable));
    }

    #[test]
    fn resolve_arrange_defaults_the_primary_to_the_current_one() {
        let snapshots = vec![
            snapshot("left", "card0-DP-1", -1920.0, 0.0, false, Some(60)),
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60)),
        ];
        let placements = resolve_arrange(
            &snapshots,
            &[ArrangeRequest {
                id: "left".to_string(),
                x: -2560,
                y: 120,
            }],
            None,
        )
        .unwrap();
        assert_eq!((placements[0].x, placements[0].y), (-2560, 120));
        assert!(!placements[0].primary);
        assert!(placements[1].primary);
    }

    #[test]
    fn resolve_arrange_switches_the_primary_and_moves_a_connector_selector() {
        let snapshots = vec![
            snapshot("left", "card0-DP-1", -1920.0, 0.0, false, Some(60)),
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60)),
        ];
        let placements = resolve_arrange(
            &snapshots,
            &[ArrangeRequest {
                id: "card0-DP-1".to_string(),
                x: -2560,
                y: 0,
            }],
            Some("card0-DP-1"),
        )
        .unwrap();
        assert_eq!(placements[0].x, -2560);
        assert!(placements[0].primary);
        assert!(!placements[1].primary);
    }

    #[test]
    fn resolve_arrange_refuses_an_unknown_display() {
        let snapshots = vec![snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60))];
        let reason = refused(resolve_arrange(
            &snapshots,
            &[ArrangeRequest {
                id: "ghost".to_string(),
                x: 0,
                y: 0,
            }],
            None,
        ));
        assert_eq!(reason, "unknown display `ghost`");
    }

    #[test]
    fn resolve_arrange_refuses_an_unknown_primary() {
        let snapshots = vec![snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60))];
        let reason = refused(resolve_arrange(&snapshots, &[], Some("ghost")));
        assert!(reason.contains("ghost"));
    }

    #[test]
    fn resolve_arrange_refuses_a_duplicate_display() {
        let snapshots = vec![
            snapshot("left", "card0-DP-1", -1920.0, 0.0, false, Some(60)),
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60)),
        ];
        let reason = refused(resolve_arrange(
            &snapshots,
            &[
                ArrangeRequest {
                    id: "left".to_string(),
                    x: -2560,
                    y: 0,
                },
                ArrangeRequest {
                    id: "card0-DP-1".to_string(),
                    x: 0,
                    y: 0,
                },
            ],
            None,
        ));
        assert!(reason.contains("more than once"));
    }

    #[test]
    fn resolve_arrange_refuses_a_display_set_without_a_primary() {
        let snapshots = vec![snapshot("main", "card0-HDMI-1", 0.0, 0.0, false, Some(60))];
        let reason = refused(resolve_arrange(&snapshots, &[], None));
        assert!(reason.contains("primary"));
    }

    #[test]
    fn resolve_arrange_refuses_a_display_set_with_several_primaries() {
        let snapshots = vec![
            snapshot("left", "card0-DP-1", -1920.0, 0.0, true, Some(60)),
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60)),
        ];
        let reason = refused(resolve_arrange(&snapshots, &[], None));
        assert!(reason.contains("primary"));
    }

    #[test]
    fn resolve_arrange_refuses_an_empty_display_set() {
        let reason = refused(resolve_arrange(&[], &[], None));
        assert!(!reason.is_empty());
    }

    #[test]
    fn resolve_config_layout_applies_positions_and_the_configured_primary() {
        let snapshots = vec![
            snapshot("left", "card0-DP-1", -1920.0, 0.0, false, Some(60)),
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60)),
        ];
        let config = BTreeMap::from([
            (
                "left".to_string(),
                LayoutPosition {
                    x: -2560,
                    y: 0,
                    primary: true,
                },
            ),
            (
                "main".to_string(),
                LayoutPosition {
                    x: 0,
                    y: 0,
                    primary: false,
                },
            ),
        ]);
        let placements = resolve_config_layout(&snapshots, &config).unwrap();
        assert_eq!(placements[0].x, -2560);
        assert!(placements[0].primary);
        assert!(!placements[1].primary);
    }

    #[test]
    fn resolve_config_layout_keeps_the_current_primary_when_none_is_configured() {
        let snapshots = vec![
            snapshot("left", "card0-DP-1", -1920.0, 0.0, false, Some(60)),
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60)),
        ];
        let config = BTreeMap::from([(
            "left".to_string(),
            LayoutPosition {
                x: -2560,
                y: 0,
                primary: false,
            },
        )]);
        let placements = resolve_config_layout(&snapshots, &config).unwrap();
        assert_eq!(placements[0].x, -2560);
        assert!(!placements[0].primary);
        assert!(placements[1].primary);
    }

    #[test]
    fn resolve_config_layout_refuses_several_configured_primaries() {
        let snapshots = vec![
            snapshot("left", "card0-DP-1", -1920.0, 0.0, false, Some(60)),
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60)),
        ];
        let config = BTreeMap::from([
            (
                "left".to_string(),
                LayoutPosition {
                    x: -1920,
                    y: 0,
                    primary: true,
                },
            ),
            (
                "main".to_string(),
                LayoutPosition {
                    x: 0,
                    y: 0,
                    primary: true,
                },
            ),
        ]);
        let reason = refused(resolve_config_layout(&snapshots, &config));
        assert!(reason.contains("primary"));
    }

    #[test]
    fn resolve_config_layout_refuses_two_keys_for_one_display() {
        let snapshots = vec![
            snapshot("left", "card0-DP-1", -1920.0, 0.0, false, Some(60)),
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60)),
        ];
        let config = BTreeMap::from([
            (
                "card0-DP-1".to_string(),
                LayoutPosition {
                    x: -2560,
                    y: 0,
                    primary: false,
                },
            ),
            (
                "left".to_string(),
                LayoutPosition {
                    x: 0,
                    y: 0,
                    primary: false,
                },
            ),
        ]);
        let reason = refused(resolve_config_layout(&snapshots, &config));
        assert!(reason.contains("more than once"));
    }

    #[test]
    fn resolve_config_layout_refuses_an_unknown_display() {
        let snapshots = vec![snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60))];
        let config = BTreeMap::from([(
            "ghost".to_string(),
            LayoutPosition {
                x: 0,
                y: 0,
                primary: false,
            },
        )]);
        let reason = refused(resolve_config_layout(&snapshots, &config));
        assert!(reason.contains("ghost"));
    }

    #[test]
    fn resolve_config_layout_refuses_when_neither_the_config_nor_the_host_names_a_primary() {
        let snapshots = vec![snapshot("main", "card0-HDMI-1", 0.0, 0.0, false, Some(60))];
        let config = BTreeMap::from([(
            "main".to_string(),
            LayoutPosition {
                x: 0,
                y: 0,
                primary: false,
            },
        )]);
        let reason = refused(resolve_config_layout(&snapshots, &config));
        assert!(reason.contains("primary"));
    }

    #[test]
    fn resolve_config_layout_refuses_an_empty_display_set() {
        let config = BTreeMap::from([(
            "main".to_string(),
            LayoutPosition {
                x: 0,
                y: 0,
                primary: false,
            },
        )]);
        let reason = refused(resolve_config_layout(&[], &config));
        assert!(!reason.is_empty());
    }

    #[test]
    fn snapshot_for_resolves_an_exact_id_then_an_exact_connector() {
        let snapshots = vec![
            snapshot("left", "card0-DP-1", -1920.0, 0.0, false, Some(60)),
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60)),
        ];
        assert_eq!(
            snapshot_for(&snapshots, "left").unwrap().handle.id(),
            "left"
        );
        assert_eq!(
            snapshot_for(&snapshots, "card0-HDMI-1")
                .unwrap()
                .handle
                .id(),
            "main"
        );
    }

    #[test]
    fn snapshot_for_refuses_an_ambiguous_id() {
        let snapshots = vec![
            snapshot("dup", "card0-DP-1", 0.0, 0.0, true, Some(60)),
            snapshot("dup", "card0-DP-2", 1920.0, 0.0, false, Some(60)),
        ];
        match snapshot_for(&snapshots, "dup").unwrap_err() {
            MonitorError::Refused { capability, reason } => {
                assert_eq!(capability, "display");
                assert!(reason.contains("ambiguous"));
                assert!(reason.contains("card0-DP-1"));
                assert!(reason.contains("card0-DP-2"));
            }
            other => panic!("expected an ambiguity refusal, got {other:?}"),
        }
    }

    #[test]
    fn snapshot_for_refuses_an_ambiguous_connector() {
        let snapshots = vec![
            snapshot("one", "card0-DP-1", 0.0, 0.0, true, Some(60)),
            snapshot("two", "card0-DP-1", 1920.0, 0.0, false, Some(60)),
        ];
        match snapshot_for(&snapshots, "card0-DP-1").unwrap_err() {
            MonitorError::Refused { reason, .. } => assert!(reason.contains("ambiguous")),
            other => panic!("expected an ambiguity refusal, got {other:?}"),
        }
    }

    #[test]
    fn snapshot_for_refuses_a_prefix_and_reports_missing_displays() {
        let snapshots = vec![snapshot("left", "card0-DP-1", 0.0, 0.0, true, Some(60))];
        assert!(matches!(
            snapshot_for(&snapshots, "le"),
            Err(MonitorError::DisplayNotFound(selector)) if selector == "le"
        ));
        assert!(matches!(
            snapshot_for(&snapshots, "card0"),
            Err(MonitorError::DisplayNotFound(selector)) if selector == "card0"
        ));
    }

    #[test]
    fn resolve_mode_selects_a_unique_match_or_an_exact_refresh() {
        let modes = vec![
            mode(1, 1920, 1080, 60),
            mode(2, 1920, 1080, 50),
            mode(3, 2560, 1440, 144),
        ];
        let unique = resolve_mode(&modes, 2560, 1440, None).unwrap();
        assert_eq!(unique.token, 3);
        let at_50 = resolve_mode(&modes, 1920, 1080, Some(50)).unwrap();
        assert_eq!(at_50.token, 2);
        let at_60 = resolve_mode(&modes, 1920, 1080, Some(60)).unwrap();
        assert_eq!(at_60.token, 1);
        let reason = refused_reason(resolve_mode(&modes, 1920, 1080, None));
        assert!(reason.contains("ambiguous without a refresh rate"));
        assert!(reason.contains("1920x1080@50"));
        assert!(reason.contains("1920x1080@60"));
        let reason = refused_reason(resolve_mode(&modes, 800, 600, None));
        assert!(reason.contains("no mode matches 800x600"));
        assert!(reason.contains("1920x1080@60"));
        assert!(reason.contains("1920x1080@50"));
        assert!(reason.contains("2560x1440@144"));
    }

    #[test]
    fn resolve_mode_refuses_colliding_rounded_refreshes() {
        let modes = vec![mode(10, 1920, 1080, 60), mode(11, 1920, 1080, 60)];
        let reason = refused_reason(resolve_mode(&modes, 1920, 1080, Some(60)));
        assert!(reason.contains("several modes match 1920x1080@60"));
        assert!(reason.contains("1920x1080@60"));
    }

    #[test]
    fn mode_lists_key_each_display_and_skip_unreadable_lists() {
        let snapshots = vec![
            snapshot("left", "card0-DP-1", -1920.0, 0.0, false, Some(60)),
            snapshot("main", "card0-HDMI-1", 0.0, 0.0, true, Some(60)),
        ];
        let modes = mode_lists(&snapshots, |handle| {
            if handle.connector() == "card0-DP-1" {
                Err(MonitorError::refused("modes", "the list is unreadable"))
            } else {
                Ok(vec![mode(1, 1280, 720, 75)])
            }
        });
        assert_eq!(modes.len(), 1);
        assert_eq!(modes["card0-HDMI-1"], vec![mode(1, 1280, 720, 75)]);
        assert!(!modes.contains_key("card0-DP-1"));
    }

    #[test]
    fn layout_position_defaults_every_missing_field() {
        let empty: LayoutPosition = serde_json::from_str("{}").unwrap();
        assert_eq!(
            empty,
            LayoutPosition {
                x: 0,
                y: 0,
                primary: false,
            }
        );
        let x_only: LayoutPosition = serde_json::from_str(r#"{"x":640}"#).unwrap();
        assert_eq!(
            x_only,
            LayoutPosition {
                x: 640,
                y: 0,
                primary: false,
            }
        );
        let partial_json = r#"{"y":-120,"primary":true}"#;
        let partial: LayoutPosition = serde_json::from_str(partial_json).unwrap();
        assert_eq!(
            partial,
            LayoutPosition {
                x: 0,
                y: -120,
                primary: true,
            }
        );
        let position = LayoutPosition {
            x: -1920,
            y: 0,
            primary: true,
        };
        let json = serde_json::to_string(&position).unwrap();
        assert_eq!(json, r#"{"x":-1920,"y":0,"primary":true}"#);
        let parsed: LayoutPosition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, position);
    }
}
