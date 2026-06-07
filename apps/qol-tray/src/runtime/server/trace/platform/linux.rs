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
            .map(|(idx, monitor)| monitor.legend_entry(idx, super::super::position_label(idx, len)))
            .collect::<Vec<_>>()
            .join(" ");
        qol_runtime::probe!("LEGEND", "mon {legend}");
    }
}

#[cfg(any(debug_assertions, test))]
struct Monitor {
    connector: String,
    x: i32,
    y: i32,
    primary: bool,
}

#[cfg(any(debug_assertions, test))]
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

#[cfg(debug_assertions)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
