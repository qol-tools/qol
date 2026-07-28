use crate::MonitorBounds;

#[derive(Debug, Clone, PartialEq)]
pub struct XrandrMonitor {
    pub connector: String,
    pub bounds: MonitorBounds,
    pub primary: bool,
}

pub fn parse_monitors(output: &str) -> Vec<XrandrMonitor> {
    output.lines().filter_map(parse_monitor_line).collect()
}

pub fn parse_monitor_line(line: &str) -> Option<XrandrMonitor> {
    let mut fields = line.split_whitespace();
    let connector = fields.next()?.to_string();
    if fields.next()? != "connected" {
        return None;
    }
    let bounds = fields.clone().find_map(parse_geometry_token)?;
    Some(XrandrMonitor {
        connector,
        bounds,
        primary: fields.any(|field| field == "primary"),
    })
}

pub fn parse_geometry_token(token: &str) -> Option<MonitorBounds> {
    let (width_raw, rest) = token.split_once('x')?;
    let width = width_raw.parse::<f32>().ok()?;
    let height_end = rest
        .char_indices()
        .find_map(|(idx, ch)| (!ch.is_ascii_digit()).then_some(idx))?;
    let height = rest.get(..height_end)?.parse::<f32>().ok()?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let offsets = rest.get(height_end..)?;
    let (x, tail) = parse_signed_offset(offsets)?;
    let (y, tail) = parse_signed_offset(tail)?;
    if !tail.is_empty() {
        return None;
    }
    Some(MonitorBounds {
        x: x as f32,
        y: y as f32,
        width,
        height,
    })
}

fn parse_signed_offset(input: &str) -> Option<(i32, &str)> {
    let bytes = input.as_bytes();
    let mut start = 0;
    if bytes.first() == Some(&b'+') && bytes.get(1) == Some(&b'-') {
        start = 1;
    }
    if !matches!(bytes.get(start), Some(b'+' | b'-')) {
        return None;
    }
    let end = input[start + 1..]
        .char_indices()
        .find_map(|(idx, ch)| (!ch.is_ascii_digit()).then_some(start + 1 + idx))
        .unwrap_or(input.len());
    if end == start + 1 {
        return None;
    }
    let value = input.get(start..end)?.parse::<i32>().ok()?;
    Some((value, input.get(end..)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    type GeometryCase = (&'static str, f32, f32, f32, f32);

    #[test]
    fn geometry_parses_signed_offsets() {
        let cases: &[GeometryCase] = &[
            ("1920x1080+0+0", 0.0, 0.0, 1920.0, 1080.0),
            ("1920x1080-1920+0", -1920.0, 0.0, 1920.0, 1080.0),
            ("1920x1080+0-1080", 0.0, -1080.0, 1920.0, 1080.0),
            ("1920x1080-1920-1080", -1920.0, -1080.0, 1920.0, 1080.0),
            ("1920x1080+-1920+0", -1920.0, 0.0, 1920.0, 1080.0),
            ("1920x1080+0+-1080", 0.0, -1080.0, 1920.0, 1080.0),
            ("1920x1080+-1920+-1080", -1920.0, -1080.0, 1920.0, 1080.0),
        ];
        for (token, x, y, width, height) in cases {
            let bounds = parse_geometry_token(token).unwrap_or_else(|| panic!("token: {token}"));
            assert_eq!(
                (bounds.x, bounds.y, bounds.width, bounds.height),
                (*x, *y, *width, *height),
                "token: {token}"
            );
        }
    }

    #[test]
    fn monitor_line_carries_connector_and_primary() {
        let monitor =
            parse_monitor_line("DP-2 connected primary 2560x1440-1920+0 (normal left)").unwrap();
        assert_eq!(monitor.connector, "DP-2");
        assert_eq!(
            (
                monitor.bounds.x,
                monitor.bounds.y,
                monitor.bounds.width,
                monitor.bounds.height,
            ),
            (-1920.0, 0.0, 2560.0, 1440.0)
        );
        assert!(monitor.primary);

        let secondary = parse_monitor_line("HDMI-0 connected 1920x1080+0+-1080 (normal)").unwrap();
        assert_eq!(secondary.connector, "HDMI-0");
        assert_eq!((secondary.bounds.x, secondary.bounds.y), (0.0, -1080.0));
        assert!(!secondary.primary);
    }

    #[test]
    fn monitor_output_skips_non_monitor_lines() {
        let output = "\
Screen 0: minimum 320 x 200, current 4480 x 1440, maximum 16384 x 16384
DP-2 connected primary 2560x1440+0+0 (normal left)
HDMI-1 disconnected (normal left inverted right x axis)
   1920x1080     60.00*+   59.93
HDMI-0 connected 1920x1080-1920+360 (normal)";
        let monitors = parse_monitors(output);
        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[0].connector, "DP-2");
        assert_eq!(monitors[1].connector, "HDMI-0");
        assert_eq!(
            (monitors[1].bounds.x, monitors[1].bounds.y),
            (-1920.0, 360.0)
        );
    }

    #[test]
    fn invalid_geometry_is_rejected() {
        let cases = [
            "1920x1080",
            "1920x1080+0",
            "1920x1080+",
            "0x1080+0+0",
            "1920x0+0+0",
            "1920x1080+0+0i",
            "1920x1080++0+0",
            "1920x1080-+0+0",
        ];
        for token in cases {
            assert!(
                parse_geometry_token(token).is_none(),
                "token should be rejected: {token}"
            );
        }
    }
}
