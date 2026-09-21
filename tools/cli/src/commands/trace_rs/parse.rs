use super::*;

pub(super) fn field<'a>(msg: &'a str, name: &str) -> Option<&'a str> {
    msg.split_whitespace().find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == name).then_some(value)
    })
}

pub(super) fn quoted_field(msg: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = msg.find(&needle)? + needle.len();
    let rest = &msg[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

pub(super) fn first_quoted(msg: &str) -> Option<String> {
    let start = msg.find('"')? + 1;
    let rest = &msg[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

pub(super) fn bracket_field(msg: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=[");
    let start = msg.find(&needle)? + needle.len();
    let rest = &msg[start..];
    let end = rest.find(']')?;
    Some(rest[..end].to_string())
}

pub(super) fn tuple_field(msg: &str, name: &str) -> Option<(i64, i64)> {
    let needle = format!("{name}=(");
    let start = msg.find(&needle)? + needle.len();
    let rest = &msg[start..];
    let end = rest.find(')')?;
    let mut parts = rest[..end].split(',');
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    Some((x, y))
}

pub(super) fn paren_field<'a>(msg: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=(");
    let start = msg.find(&needle)? + needle.len();
    let rest = &msg[start..];
    let end = rest.find(')')?;
    Some(&rest[..end])
}

pub(super) fn launcher_pos_size(msg: &str) -> Option<(i64, i64, i64, i64)> {
    let (x, y) = tuple_field(msg, "pos")?;
    let size = field(msg, "size")?;
    let (w, h) = size.split_once('x')?;
    Some((x, y, w.parse().ok()?, h.parse().ok()?))
}

pub(super) fn launcher_window(msg: &str) -> String {
    let Some(value) = paren_field(msg, "win") else {
        return "win=?".to_string();
    };
    let mut parts = value.split(',');
    let Some(x) = parts.next() else {
        return "win=?".to_string();
    };
    let Some(y) = parts.next() else {
        return "win=?".to_string();
    };
    let Some(size) = parts.next() else {
        return "win=?".to_string();
    };
    format!("{size}@({x},{y})")
}

pub(super) fn sequence(msg: &str) -> Option<&str> {
    msg.split_whitespace()
        .find_map(|part| part.strip_prefix('#'))
        .filter(|seq| seq.chars().all(|ch| ch.is_ascii_digit()))
}

pub(super) fn arrow_status(msg: &str) -> Option<(String, String)> {
    let (_, rest) = msg.split_once(" -> ")?;
    let Some((comparison, status_part)) = rest.split_once(" (") else {
        return Some((rest.trim().to_string(), "?".to_string()));
    };
    Some((
        comparison.trim().to_string(),
        status_part.trim_end_matches(')').to_string(),
    ))
}

pub(super) fn ewmh_payload(msg: &str) -> String {
    let Some(source) = field(msg, "source") else {
        return String::new();
    };
    let Some(timestamp) = field(msg, "timestamp") else {
        return String::new();
    };
    let active = field(msg, "requester_active")
        .or_else(|| field(msg, "requestor_active"))
        .unwrap_or("?");
    format!(
        "{COLOR_DIM} (EWMH: source={source}, timestamp={timestamp}, active={active}){COLOR_RESET}"
    )
}

pub(super) struct PythonGhostDump<'a> {
    pub(super) ctx: &'a str,
    pub(super) title: &'a str,
    pub(super) alpha: &'a str,
    pub(super) level: &'a str,
    pub(super) mouse_ignored: &'a str,
    pub(super) frame: &'a str,
}

pub(super) struct PythonCycle<'a> {
    pub(super) method: &'a str,
    pub(super) from: &'a str,
    pub(super) to: &'a str,
    pub(super) count: &'a str,
    pub(super) app: &'a str,
    pub(super) title: &'a str,
    pub(super) elapsed_ms: &'a str,
}

pub(super) fn parse_python_cycle(msg: &str) -> Option<PythonCycle<'_>> {
    let method = field(msg, "method")?;
    let from = field(msg, "from")?;
    let to = field(msg, "to")?;
    let count = field(msg, "count")?;
    if count.is_empty() || !count.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    let (_, rest) = msg.split_once(" to_app=\"")?;
    let app_end = rest.find('"')?;
    let app = &rest[..app_end];
    let rest = rest[app_end + 1..].trim_start();
    let rest = rest.strip_prefix("to_title=\"")?;
    let title_end = rest.find('"')?;
    let title = &rest[..title_end];
    let rest = rest[title_end + 1..].trim_start();
    let elapsed_ms = rest.strip_prefix("elapsed_ms=")?;
    let elapsed_ms = elapsed_ms.split_whitespace().next().unwrap_or(elapsed_ms);
    if elapsed_ms.is_empty() || !elapsed_ms.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    Some(PythonCycle {
        method,
        from,
        to,
        count,
        app,
        title,
        elapsed_ms,
    })
}

pub(super) fn parse_python_ghost_dump(msg: &str) -> Option<PythonGhostDump<'_>> {
    let needle = "ctx=(";
    let ctx_start = msg.find(needle)? + needle.len();
    let ctx_rest = &msg[ctx_start..];
    let ctx_end = ctx_rest.find(')')?;
    let ctx = &ctx_rest[..ctx_end];
    let rest = ctx_rest[ctx_end + 1..].trim_start();

    let rest = rest.strip_prefix("title=\"")?;
    let title_end = rest.find('"')?;
    let title = &rest[..title_end];
    let mut tokens = rest[title_end + 1..].split_whitespace();

    let alpha = tokens.next()?.strip_prefix("alpha=")?;
    if alpha.is_empty() || !alpha.chars().all(|ch| ch.is_ascii_digit() || ch == '.') {
        return None;
    }

    let level = tokens.next()?.strip_prefix("level=")?;
    let level_digits = level.strip_prefix('-').unwrap_or(level);
    if level_digits.is_empty() || !level_digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    let mouse_ignored = tokens.next()?.strip_prefix("mouse_ignored=")?;
    if mouse_ignored.is_empty()
        || !mouse_ignored
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }

    let frame = tokens.next()?.strip_prefix("frame=")?;
    if frame.is_empty() {
        return None;
    }

    Some(PythonGhostDump {
        ctx,
        title,
        alpha,
        level,
        mouse_ignored,
        frame,
    })
}

pub(super) fn hide_opacity(msg: &str) -> Option<(f64, Option<&str>)> {
    msg.split_whitespace()
        .collect::<Vec<_>>()
        .windows(4)
        .find_map(|tokens| {
            let title = tokens[0].strip_prefix("title=")?;
            if title.is_empty() {
                return None;
            }
            let wid = tokens[1].strip_prefix("wid=")?;
            if wid.is_empty() || !wid.chars().all(|ch| ch.is_ascii_digit()) {
                return None;
            }
            let path = tokens[2].strip_prefix("path=")?;
            if path.is_empty() {
                return None;
            }
            let opacity = tokens[3].strip_prefix("opacity=")?;
            Some((parse_python_float_prefix(opacity)?, Some(path)))
        })
}

pub(super) fn show_opacity(msg: &str) -> Option<f64> {
    msg.split_whitespace()
        .collect::<Vec<_>>()
        .windows(3)
        .find_map(|tokens| {
            let title = tokens[0].strip_prefix("title=")?;
            if title.is_empty() {
                return None;
            }
            let wid = tokens[1].strip_prefix("wid=")?;
            if wid.is_empty() || !wid.chars().all(|ch| ch.is_ascii_digit()) {
                return None;
            }
            let opacity = tokens[2].strip_prefix("cleared_opacity->")?;
            parse_python_float_prefix(opacity)
        })
}

pub(super) fn parse_f64(value: &str) -> Option<f64> {
    value.parse::<f64>().ok()
}

pub(super) fn parse_python_float_prefix(value: &str) -> Option<f64> {
    let text = value
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect::<String>();
    if text.is_empty() {
        return None;
    }
    text.parse::<f64>().ok()
}

pub(super) fn parse_ghost_window(msg: &str, ts_ms: u64) -> Option<GhostWindow> {
    let title = field(msg, "title")?.to_string();
    let owner_pid = field(msg, "owner_pid").unwrap_or_default().to_string();
    let (x, y) = tuple_field(msg, "pos")?;
    let size = field(msg, "size")?;
    let (_w, _h) = size.split_once('x')?;
    let opacity = match field(msg, "opacity")? {
        "unset" => 1.0,
        value => parse_f64(value)?,
    };
    Some(GhostWindow {
        sample_ts_ms: ts_ms,
        title,
        opacity,
        role: field(msg, "role")?.to_string(),
        map_state: field(msg, "map")?.to_string(),
        owner_pid,
        x,
        y,
    })
}

pub(super) fn parse_qol_title_origin(title: &str) -> Option<(i64, i64)> {
    let (_, rest) = title.split_once('@')?;
    let mut parts = rest.split(',');
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    Some((x, y))
}

pub(super) fn reason(msg: &str) -> &str {
    field(msg, "reason").unwrap_or("?")
}

pub(super) fn reason_suffix(msg: &str) -> String {
    match reason(msg) {
        "?" => String::new(),
        reason => format!(" {COLOR_DIM}(why: {reason}){COLOR_RESET}"),
    }
}

pub(super) fn title_contains_match(left: &str, right: &str) -> bool {
    let left = left.to_lowercase();
    let right = right.to_lowercase();
    left.contains(&right) || right.contains(&left)
}

pub(super) fn parse_at_bounds(token: &str) -> Option<(i64, i64, i64, i64)> {
    let (_, rest) = token.split_once('@')?;
    let (x, rest) = rest.split_once(',')?;
    let (y, rest) = rest.split_once(',')?;
    let rest = rest.trim_end_matches(|ch: char| !ch.is_ascii_digit());
    let (w, h) = rest.split_once('x')?;
    Some((
        x.parse().ok()?,
        y.parse().ok()?,
        w.parse().ok()?,
        h.parse().ok()?,
    ))
}

pub(super) fn parse_monitor_bounds_debug(msg: &str) -> Vec<(i64, i64, i64, i64)> {
    let mut bounds = Vec::new();
    let mut rest = msg;
    while let Some(start) = rest.find("MonitorBounds {") {
        let block_start = start + "MonitorBounds {".len();
        let tail = &rest[block_start..];
        let Some(end) = tail.find('}') else {
            break;
        };
        let block = &tail[..end];
        if let (Some(x), Some(y), Some(w), Some(h)) = (
            debug_number_field(block, "x"),
            debug_number_field(block, "y"),
            debug_number_field(block, "width"),
            debug_number_field(block, "height"),
        ) {
            bounds.push((x, y, w, h));
        }
        rest = &tail[end + 1..];
    }
    bounds
}

pub(super) fn debug_number_field(block: &str, name: &str) -> Option<i64> {
    let marker = format!("{name}:");
    let tail = block[block.find(&marker)? + marker.len()..].trim_start();
    let value = tail
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+'))
        .collect::<String>();
    if value.is_empty() {
        return None;
    }
    value.parse::<f64>().ok().map(|value| value as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_monitor_bounds_from_trace_token() {
        assert_eq!(
            parse_at_bounds("target=0,0@0,0,1800x1169"),
            Some((0, 0, 1800, 1169))
        );
    }

    #[test]
    fn hide_opacity_parses_unmap_probe_but_not_miss_probe() {
        assert_eq!(
            hide_opacity(
                "title=qol-alt-tab-picker@0,0,100x100 wid=5 path=unmap opacity=0 compositor=true opacity_ok=true passthrough=true unmapped=true attempts=1 flush=true reason=boot"
            ),
            Some((0.0, Some("unmap")))
        );
        assert_eq!(
            hide_opacity("title=qol-alt-tab-picker@0,0,100x100 wid=NONE attempts=6 reason=boot"),
            None
        );
    }

    #[test]
    fn show_opacity_parses_cleared_field_immediately_after_wid() {
        assert_eq!(
            show_opacity(
                "title=qol-launcher@0,0,100x100 wid=5 cleared_opacity->1 presentation=Overlay state=true source=2 reason=show"
            ),
            Some(1.0)
        );
        assert_eq!(
            show_opacity(
                "title=qol-launcher@0,0,100x100 wid=5 presentation=Overlay cleared_opacity->1 state=true reason=show"
            ),
            None
        );
    }

    #[test]
    fn parses_monitor_bounds_from_debug_shape() {
        assert_eq!(
            parse_monitor_bounds_debug(
                "active=qol-launcher@0,0,1800x1169 bounds=MonitorBounds { x: -1920.0, y: 0.0, width: 1920.0, height: 1080.0 }"
            ),
            vec![(-1920, 0, 1920, 1080)]
        );
    }
}
