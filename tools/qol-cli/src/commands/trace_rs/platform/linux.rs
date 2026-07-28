use super::TracePlatformOps;
use std::process::Command;

pub(super) struct Platform;

impl TracePlatformOps for Platform {
    fn process_name(&self, pid: &str) -> Option<String> {
        super::unix::process_name(pid)
    }

    fn initial_monitor_bounds(&self) -> Vec<(i64, i64, i64, i64)> {
        let Some(output) = Command::new("xrandr")
            .arg("--current")
            .output()
            .ok()
            .filter(|output| output.status.success())
        else {
            return Vec::new();
        };
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(parse_xrandr_geometry_line)
            .collect()
    }
}

fn parse_xrandr_geometry_line(line: &str) -> Option<(i64, i64, i64, i64)> {
    if !line.contains(" connected") {
        return None;
    }
    line.split_whitespace()
        .find_map(parse_xrandr_geometry_token)
}

fn parse_xrandr_geometry_token(token: &str) -> Option<(i64, i64, i64, i64)> {
    let (width, rest) = token.split_once('x')?;
    let coord_start = rest.find(['+', '-'])?;
    let height = &rest[..coord_start];
    let coords = &rest[coord_start..];
    let (x, coords) = parse_signed_coord(coords)?;
    let (y, _) = parse_signed_coord(coords)?;
    Some((x, y, width.parse().ok()?, height.parse().ok()?))
}

fn parse_signed_coord(input: &str) -> Option<(i64, &str)> {
    let (sign, rest) = match input.as_bytes().first()? {
        b'+' => {
            let rest = &input[1..];
            if let Some(rest) = rest.strip_prefix('-') {
                (-1, rest)
            } else {
                (1, rest)
            }
        }
        b'-' => (-1, &input[1..]),
        _ => return None,
    };
    let digits = rest
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    let value = digits.parse::<i64>().ok()? * sign;
    Some((value, &rest[digits.len()..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_monitor_bounds_from_xrandr_connected_line() {
        assert_eq!(
            parse_xrandr_geometry_line(
                "DP-2 connected primary 1800x1169+0+0 (normal left inverted right x axis y axis) 345mm x 223mm"
            ),
            Some((0, 0, 1800, 1169))
        );
        assert_eq!(
            parse_xrandr_geometry_line(
                "HDMI-1 connected 1920x1080-1920+0 (normal left inverted right x axis y axis) 600mm x 340mm"
            ),
            Some((-1920, 0, 1920, 1080))
        );
        assert_eq!(parse_xrandr_geometry_line("DP-3 disconnected"), None);
    }
}
