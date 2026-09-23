use super::*;

pub(super) struct RawLine {
    pub(super) ts_ms: u64,
    pub(super) pid: String,
    pub(super) tag: String,
    pub(super) msg: String,
}

pub(super) fn parse_raw_line(line: &str) -> Option<RawLine> {
    let (ts, rest) = line.split_once(" pid=")?;
    let ts_ms = ts.parse().ok()?;
    let (pid, rest) = rest.split_once(' ')?;
    let (tag, msg) = rest.split_once(' ')?;
    Some(RawLine {
        ts_ms,
        pid: pid.to_string(),
        tag: tag.to_string(),
        msg: msg.to_string(),
    })
}

pub(super) struct Event {
    pub(super) ts_ms: u64,
    pub(super) ts: String,
    pub(super) tag: String,
    pub(super) source: String,
    pub(super) filter_source: Option<String>,
    pub(super) text: String,
}

#[derive(Clone, Debug)]
pub(super) struct PendingActivation {
    pub(super) ts_ms: u64,
    pub(super) seq: Option<String>,
    pub(super) wid: String,
    pub(super) title: String,
    pub(super) source: String,
    pub(super) confirmed_front: bool,
}

#[derive(Clone, Debug)]
pub(super) struct OpacityWrite {
    pub(super) op: f64,
    pub(super) reason: String,
    pub(super) ts_ms: u64,
    pub(super) prev_op: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum OpacityClassification {
    Redundant,
    Revert {
        previous_reason: String,
        age_ms: u64,
    },
}

#[derive(Clone, Debug)]
pub(super) struct GhostWindow {
    pub(super) sample_ts_ms: u64,
    pub(super) title: String,
    pub(super) opacity: f64,
    pub(super) role: String,
    pub(super) map_state: String,
    pub(super) owner_pid: String,
    pub(super) x: i64,
    pub(super) y: i64,
}

#[derive(Default)]
pub(super) struct TraceStats {
    pub(super) focus_req: usize,
    pub(super) focus_ok: usize,
    pub(super) focus_misdirect: usize,
    pub(super) focus_timeout: usize,
    pub(super) supersede: usize,
    pub(super) divergence: usize,
    pub(super) oscillation: usize,
    pub(super) latencies: Vec<u64>,
    pub(super) focus_history: Vec<(u64, String)>,
    pub(super) last_divergence: Option<String>,
}

#[derive(Default)]
pub(super) struct OpacityWaste {
    pub(super) writes: usize,
    pub(super) redundant: usize,
    pub(super) reverts: usize,
    pub(super) by_reason: HashMap<String, usize>,
    pub(super) reason_order: Vec<String>,
    pub(super) redundant_by_reason: HashMap<String, usize>,
    pub(super) revert_pairs: HashMap<String, usize>,
    pub(super) revert_pair_order: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_raw_line_reads_probe_shape() {
        let raw = parse_raw_line(
            "1781848506980 pid=62404 CLI_SESSIONS_KITTEN args=[\"@\", \"ls\"] ok=true",
        )
        .expect("raw line");
        assert_eq!(raw.ts_ms, 1781848506980);
        assert_eq!(raw.pid, "62404");
        assert_eq!(raw.tag, "CLI_SESSIONS_KITTEN");
        assert_eq!(raw.msg, "args=[\"@\", \"ls\"] ok=true");
    }
}
