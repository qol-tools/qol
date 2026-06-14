pub(super) fn print_monitor_legend() {
    #[cfg(debug_assertions)]
    {
        let monitors = xrandr_monitors();
        if monitors.is_empty() {
            return;
        }
        let len = monitors.len();
        let legend = monitors
            .iter()
            .enumerate()
            .map(|(idx, monitor)| {
                legend_entry(monitor, idx, super::super::position_label(idx, len))
            })
            .collect::<Vec<_>>()
            .join(" ");
        qol_runtime::probe!("LEGEND", "mon {legend}");
    }
}

#[cfg(any(debug_assertions, test))]
fn legend_entry(monitor: &qol_runtime::xrandr::XrandrMonitor, idx: usize, label: &str) -> String {
    let primary = if monitor.primary { "*primary" } else { "" };
    format!(
        "idx{idx}={}@{},{} \"{label}\"{primary}",
        monitor.connector, monitor.bounds.x as i32, monitor.bounds.y as i32
    )
}

#[cfg(debug_assertions)]
fn xrandr_monitors() -> Vec<qol_runtime::xrandr::XrandrMonitor> {
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
    let mut monitors = qol_runtime::xrandr::parse_monitors(&stdout);
    monitors.sort_by_key(|monitor| (monitor.bounds.x as i32, monitor.bounds.y as i32));
    monitors
}
