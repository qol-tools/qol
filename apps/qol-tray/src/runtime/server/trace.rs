use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};
use qol_runtime::MonitorBounds;

mod platform;

pub(super) fn print_monitor_legend() {
    platform::print_monitor_legend();
}

pub(super) fn subscribed(clean_id: &str, events: &[RuntimeEventKind], replayed_idx: Option<usize>) {
    #[cfg(debug_assertions)]
    {
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

    #[cfg(not(debug_assertions))]
    let _ = (clean_id, events, replayed_idx);
}

pub(super) fn publish_summary(
    events: &[RuntimeEvent],
    subscriber_results: &[(String, bool, bool)],
    amc_interested: &[String],
    armed_lifelines: &[String],
    monitors: &[MonitorBounds],
) {
    #[cfg(debug_assertions)]
    {
        for event in events {
            if let RuntimeEvent::MonitorsChanged { monitors } = event {
                qol_runtime::probe!("PUBLISH_MONITORS", "n={}", monitors.len());
            }
        }
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

    #[cfg(not(debug_assertions))]
    let _ = (
        events,
        subscriber_results,
        amc_interested,
        armed_lifelines,
        monitors,
    );
}

#[cfg(debug_assertions)]
fn boot_amc() -> bool {
    static IS_BOOT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
    IS_BOOT.swap(false, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(debug_assertions)]
fn monitor_label(active: Option<&MonitorBounds>, monitors: &[MonitorBounds]) -> &'static str {
    let Some(active) = active else {
        return "C";
    };
    let mut sorted = monitors.to_vec();
    sorted.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    let pos = sorted.iter().position(|m| m == active).unwrap_or(0);
    position_label(pos, sorted.len())
}

#[cfg(debug_assertions)]
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

#[cfg(debug_assertions)]
fn strip(plugin_id: &str) -> &str {
    plugin_id.strip_prefix("plugin-").unwrap_or(plugin_id)
}

#[cfg(any(debug_assertions, test))]
fn position_label(idx: usize, len: usize) -> &'static str {
    match (idx, len) {
        (_, 1) => "C",
        (0, _) => "L",
        (i, n) if i == n - 1 => "R",
        _ => "C",
    }
}

#[cfg(test)]
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
}
