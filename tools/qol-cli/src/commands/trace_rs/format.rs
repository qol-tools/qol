use super::*;

pub(super) fn format_group(events: &[Event], details: bool) -> String {
    let root = &events[0];
    let span_ms = events
        .last()
        .map(|last| last.ts_ms.saturating_sub(root.ts_ms))
        .unwrap_or(0);
    let latency = if span_ms > 0 {
        format!(" {COLOR_TIME}(span: {span_ms}ms){COLOR_RESET}")
    } else {
        String::new()
    };
    let src_tag = format!(
        "{}[{}]{COLOR_RESET} ",
        hash_color(&root.source),
        root.source
    );
    let mut out = String::new();
    if events.len() == 1 {
        let _ = writeln!(
            out,
            "{COLOR_TIME}[{}]{COLOR_RESET} ── {src_tag}{}{}",
            root.ts, root.text, latency
        );
        return out;
    }
    if !details {
        let _ = writeln!(
            out,
            "{COLOR_TIME}[{}]{COLOR_RESET} ── {src_tag}{}{} {}",
            root.ts,
            root.text,
            latency,
            detail_suffix(events.len() - 1)
        );
        return out;
    }
    let _ = writeln!(
        out,
        "{COLOR_TIME}[{}]{COLOR_RESET} ┌── {src_tag}{}{}",
        root.ts, root.text, latency
    );
    for (idx, event) in events.iter().enumerate().skip(1) {
        let connector = if idx == events.len() - 1 {
            "└── "
        } else {
            "├── "
        };
        let src_tag = format!(
            "{}[{}]{COLOR_RESET} ",
            hash_color(&event.source),
            event.source
        );
        let _ = writeln!(
            out,
            "{COLOR_TIME}[{}]{COLOR_RESET} │   {connector}{src_tag}{}",
            event.ts, event.text
        );
    }
    out
}

pub(super) fn detail_suffix(hidden_count: usize) -> String {
    let noun = if hidden_count == 1 {
        "detail"
    } else {
        "details"
    };
    format!("{COLOR_DIM}(+{hidden_count} {noun}){COLOR_RESET}")
}

pub(super) fn format_timestamp(ms: u64) -> String {
    let Some(dt) = Local.timestamp_millis_opt(ms as i64).single() else {
        return ms.to_string();
    };
    dt.format("%H:%M:%S.%3f").to_string()
}

pub(super) fn hash_color(name: &str) -> &'static str {
    const COLORS: [&str; 8] = [
        "\x1b[1;34m",
        "\x1b[1;35m",
        "\x1b[1;36m",
        "\x1b[1;32m",
        "\x1b[1;94m",
        "\x1b[1;95m",
        "\x1b[1;96m",
        "\x1b[1;92m",
    ];
    if matches!(name, "host" | "qol-tray" | "tray") {
        return COLOR_WARN;
    }
    let mut hash = 0i64;
    for ch in name.chars() {
        hash = ch as i64 + ((hash << 5) - hash);
    }
    COLORS[hash.unsigned_abs() as usize % COLORS.len()]
}

pub(super) fn format_python_float(value: &str) -> String {
    let Some(parsed) = value.parse::<f64>().ok() else {
        return value.to_string();
    };
    if parsed.fract() == 0.0 {
        format!("{parsed:.1}")
    } else {
        parsed.to_string()
    }
}

pub(super) fn path_suffix(path: Option<&str>) -> String {
    match path {
        Some("compositor" | "hidden" | "invisible" | "rest" | "unmap") | None => String::new(),
        Some(path) => format!(" {COLOR_DIM}(path: {path}){COLOR_RESET}"),
    }
}

pub(super) fn churn_suffix(classification: Option<&OpacityClassification>) -> String {
    match classification {
        Some(OpacityClassification::Revert {
            previous_reason,
            age_ms,
        }) => {
            format!(" {COLOR_FAIL}⟲ REVERT {previous_reason}@{age_ms}ms{COLOR_RESET}")
        }
        _ => String::new(),
    }
}

pub(super) fn format_opacity(value: f64) -> String {
    if opacity_eq(value.fract().abs(), 0.0) {
        return format!("{value:.1}");
    }
    let mut text = format!("{value:.3}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    text
}

pub(super) fn opacity_eq(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.001
}

pub(super) fn latency_color(value_ms: u64) -> &'static str {
    if value_ms > 100 {
        COLOR_FAIL
    } else if value_ms > 50 {
        COLOR_WARN
    } else {
        COLOR_OK
    }
}

pub(super) fn percentile(values: &[u64], p: u64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let rank = ((p as f64 / 100.0) * (ordered.len().saturating_sub(1) as f64)).round() as usize;
    ordered[rank.min(ordered.len() - 1)]
}

pub(super) fn increment_count(map: &mut HashMap<String, usize>, key: &str) {
    *map.entry(key.to_string()).or_insert(0) += 1;
}

pub(super) fn increment_ordered_count(
    map: &mut HashMap<String, usize>,
    order: &mut Vec<String>,
    key: &str,
) {
    if !map.contains_key(key) {
        order.push(key.to_string());
    }
    increment_count(map, key);
}

pub(super) fn sorted_counts(
    map: &HashMap<String, usize>,
    order: &[String],
) -> Vec<(String, usize)> {
    let mut counts = map
        .iter()
        .map(|(key, count)| (key.clone(), *count))
        .collect::<Vec<_>>();
    counts.sort_by(|(left_key, left_count), (right_key, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| insertion_rank(order, left_key).cmp(&insertion_rank(order, right_key)))
    });
    counts
}

pub(super) fn insertion_rank(order: &[String], key: &str) -> usize {
    order
        .iter()
        .position(|candidate| candidate == key)
        .unwrap_or(usize::MAX)
}

pub(super) fn winact_outcome_color(outcome: &str) -> &'static str {
    match outcome {
        "ok" => COLOR_OK,
        "fail" | "err" => COLOR_FAIL,
        _ => COLOR_DIM,
    }
}

pub(super) fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut text = value.chars().take(max).collect::<String>();
    text.push_str("...");
    text
}

pub(super) fn active_status(age_ms: u64) -> String {
    if age_ms >= 1500 {
        format!("{COLOR_FAIL}(STALE){COLOR_RESET}")
    } else {
        format!("{COLOR_OK}(ACTIVE){COLOR_RESET}")
    }
}
