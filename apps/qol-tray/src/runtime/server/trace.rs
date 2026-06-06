use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};
use qol_runtime::MonitorBounds;

#[cfg(target_os = "linux")]
pub(super) fn print_monitor_legend() {
    let monitors = xrandr_monitors();
    if monitors.is_empty() {
        return;
    }
    let len = monitors.len();
    let legend = monitors
        .iter()
        .enumerate()
        .map(|(idx, monitor)| monitor.legend_entry(idx, position_label(idx, len)))
        .collect::<Vec<_>>()
        .join(" ");
    qol_runtime::probe!("LEGEND", "mon {legend}");
}

#[cfg(not(target_os = "linux"))]
pub(super) fn print_monitor_legend() {}

pub(super) fn subscribed(clean_id: &str, events: &[RuntimeEventKind], replayed_idx: Option<usize>) {
    let interests = events
        .iter()
        .map(|kind| format!("{kind:?}"))
        .collect::<Vec<_>>()
        .join(",");
    let replay = match replayed_idx {
        Some(idx) => format!(" -> host sticky-replay AMC idx={idx}"),
        None => String::new(),
    };
    qol_runtime::probe!(
        "SUBSCRIBE",
        "plugin={clean_id} interests=[{interests}]{replay}"
    );
}

pub(super) fn publish_summary(
    events: &[RuntimeEvent],
    subscriber_results: &[(String, bool, bool)],
    amc_interested: &[String],
    armed_lifelines: &[String],
    monitors: &[MonitorBounds],
) {
    for event in events {
        let RuntimeEvent::ActiveMonitorChanged {
            monitor_idx,
            monitor,
        } = event
        else {
            continue;
        };
        let idx = monitor_idx.unwrap_or(0);
        let name = monitor_label(monitor.as_ref(), monitors);
        let is_boot = boot_amc();
        let (delivered, missed) =
            delivery_split(subscriber_results, amc_interested, armed_lifelines);
        qol_runtime::probe!(
            "PUBLISH",
            "idx={idx} \"{name}\" is_boot={is_boot} -> delivered=[{}] missed=[{}]",
            delivered.join(", "),
            missed.join(", ")
        );
    }
}

fn boot_amc() -> bool {
    static IS_BOOT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
    IS_BOOT.swap(false, std::sync::atomic::Ordering::Relaxed)
}

fn monitor_label(active: Option<&MonitorBounds>, monitors: &[MonitorBounds]) -> &'static str {
    let Some(active) = active else {
        return "C";
    };
    let mut sorted = monitors.to_vec();
    sorted.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    let pos = sorted.iter().position(|m| m == active).unwrap_or(0);
    position_label(pos, sorted.len())
}

fn delivery_split(
    subscriber_results: &[(String, bool, bool)],
    amc_interested: &[String],
    armed_lifelines: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut delivered = Vec::new();
    let mut missed = Vec::new();
    let mut accounted = std::collections::HashSet::new();

    for (plugin_id, success, amc_delivered) in subscriber_results {
        if *amc_delivered {
            delivered.push(strip(plugin_id).to_string());
            accounted.insert(plugin_id);
        } else if !success && amc_interested.contains(plugin_id) {
            missed.push(format!("{}:failed", strip(plugin_id)));
            accounted.insert(plugin_id);
        }
    }
    for plugin_id in armed_lifelines {
        if !accounted.contains(plugin_id) {
            missed.push(format!("{}:unsubscribed", strip(plugin_id)));
        }
    }

    delivered.sort();
    missed.sort();
    (delivered, missed)
}

fn strip(plugin_id: &str) -> &str {
    plugin_id.strip_prefix("plugin-").unwrap_or(plugin_id)
}

#[cfg(target_os = "linux")]
struct Monitor {
    connector: String,
    x: i32,
    y: i32,
    primary: bool,
}

#[cfg(target_os = "linux")]
impl Monitor {
    fn parse(line: &str) -> Option<Self> {
        if !line.contains(" connected") {
            return None;
        }
        let connector = line.split_whitespace().next()?.to_string();
        let geometry = line
            .split_whitespace()
            .find(|field| field.contains('x') && field.contains('+'))?;
        let (_resolution, offsets) = geometry.split_once('+')?;
        let (x, y) = offsets.split_once('+')?;
        Some(Self {
            connector,
            x: x.parse().ok()?,
            y: y.parse().ok()?,
            primary: line.contains(" primary"),
        })
    }

    fn legend_entry(&self, idx: usize, label: &str) -> String {
        let primary = if self.primary { "*primary" } else { "" };
        format!(
            "idx{idx}={}@{},{} \"{label}\"{primary}",
            self.connector, self.x, self.y
        )
    }
}

#[cfg(target_os = "linux")]
fn xrandr_monitors() -> Vec<Monitor> {
    let Ok(output) = std::process::Command::new("xrandr")
        .arg("--current")
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut monitors: Vec<Monitor> = stdout.lines().filter_map(Monitor::parse).collect();
    monitors.sort_by_key(|monitor| (monitor.x, monitor.y));
    monitors
}

fn position_label(idx: usize, len: usize) -> &'static str {
    match (idx, len) {
        (_, 1) => "C",
        (0, _) => "L",
        (i, n) if i == n - 1 => "R",
        _ => "C",
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn position_label_maps_index_to_left_center_right() {
        let cases = [
            ((0, 1), "C"),
            ((0, 2), "L"),
            ((1, 2), "R"),
            ((0, 3), "L"),
            ((1, 3), "C"),
            ((2, 3), "R"),
            ((0, 4), "L"),
            ((2, 4), "C"),
            ((3, 4), "R"),
        ];
        for ((idx, len), expected) in cases {
            assert_eq!(position_label(idx, len), expected, "idx={idx} len={len}");
        }
    }

    #[test]
    fn parse_reads_connector_offsets_and_primary() {
        let primary =
            Monitor::parse("DP-2 connected primary 2560x1440+1920+0 (normal left)").unwrap();
        assert_eq!(primary.connector, "DP-2");
        assert_eq!((primary.x, primary.y), (1920, 0));
        assert!(primary.primary);

        let secondary = Monitor::parse("HDMI-0 connected 1920x1080+4480+360 (normal)").unwrap();
        assert_eq!((secondary.x, secondary.y), (4480, 360));
        assert!(!secondary.primary);
    }

    #[test]
    fn parse_skips_non_monitor_lines() {
        let cases = [
            "HDMI-1 disconnected (normal left inverted right x axis)",
            "Screen 0: minimum 320 x 200, current 4480 x 1440, maximum 16384 x 16384",
            "   1920x1080     60.00*+   59.93",
        ];
        for line in cases {
            assert!(Monitor::parse(line).is_none(), "line: {line}");
        }
    }
}
