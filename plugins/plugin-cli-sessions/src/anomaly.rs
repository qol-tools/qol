use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::status::Status;
use crate::strategy::Phase;

const RING_CAP: usize = 6;
const FLAP_SECS: u64 = 9;

#[derive(Debug, Clone)]
pub struct Frame {
    pub ts: u64,
    pub title: String,
    pub screen: Option<String>,
    pub phase: Phase,
    pub status: Status,
}

#[derive(Debug)]
pub struct Anomaly {
    pub window_id: u64,
    pub kind: &'static str,
    pub dwell_secs: u64,
    pub frames: Vec<Frame>,
}

#[derive(Default)]
struct WindowState {
    ring: VecDeque<Frame>,
    needs_you_since: Option<u64>,
}

pub struct AnomalyRecorder {
    ring_cap: usize,
    flap_secs: u64,
    windows: HashMap<u64, WindowState>,
}

impl AnomalyRecorder {
    pub fn new(ring_cap: usize, flap_secs: u64) -> Self {
        Self {
            ring_cap,
            flap_secs,
            windows: HashMap::new(),
        }
    }

    /// Feed one observed frame. Returns an [`Anomaly`] when this frame closes a
    /// short-lived `NeedsYou` - the session asked for attention then released it
    /// on its own within `flap_secs`, which is almost always a misread.
    pub fn note(&mut self, window_id: u64, frame: Frame) -> Option<Anomaly> {
        let cap = self.ring_cap;
        let flap_secs = self.flap_secs;
        let state = self.windows.entry(window_id).or_default();

        let ts = frame.ts;
        let is_needs_you = frame.status == Status::NeedsYou;
        state.ring.push_back(frame);
        while state.ring.len() > cap {
            state.ring.pop_front();
        }

        match (is_needs_you, state.needs_you_since) {
            (true, None) => {
                state.needs_you_since = Some(ts);
                None
            }
            (true, Some(_)) => None,
            (false, Some(since)) => {
                state.needs_you_since = None;
                let dwell = ts.saturating_sub(since);
                (dwell <= flap_secs).then(|| Anomaly {
                    window_id,
                    kind: "needs_you_flap",
                    dwell_secs: dwell,
                    frames: state.ring.iter().cloned().collect(),
                })
            }
            (false, None) => None,
        }
    }

    pub fn forget(&mut self, window_id: u64) {
        self.windows.remove(&window_id);
    }
}

pub fn dump(dir: &Path, anomaly: &Anomaly) -> std::io::Result<PathBuf> {
    let stamp = anomaly.frames.last().map(|f| f.ts).unwrap_or(0);
    let target = dir.join(format!("{}_win{}", stamp, anomaly.window_id));
    std::fs::create_dir_all(&target)?;
    let mut frame_index = Vec::new();
    for (i, frame) in anomaly.frames.iter().enumerate() {
        let file = format!("frame_{i:02}.txt");
        if let Some(screen) = &frame.screen {
            std::fs::write(target.join(&file), screen)?;
        }
        frame_index.push(serde_json::json!({
            "file": file,
            "ts": frame.ts,
            "title": frame.title,
            "phase": format!("{:?}", frame.phase),
            "status": format!("{:?}", frame.status),
        }));
    }
    let report = serde_json::json!({
        "window_id": anomaly.window_id,
        "kind": anomaly.kind,
        "dwell_secs": anomaly.dwell_secs,
        "frames": frame_index,
    });
    std::fs::write(
        target.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap_or_default(),
    )?;
    Ok(target)
}

static RECORDER: OnceLock<Option<Mutex<AnomalyRecorder>>> = OnceLock::new();

fn enabled_dir() -> Option<PathBuf> {
    let flag = std::env::var("CLI_SESSIONS_RECORD_ANOMALIES").unwrap_or_default();
    if !matches!(flag.as_str(), "1" | "true" | "yes") {
        return None;
    }
    match std::env::var("CLI_SESSIONS_ANOMALY_DIR") {
        Ok(d) if !d.is_empty() => Some(PathBuf::from(d)),
        _ => crate::paths::anomalies_dir(),
    }
}

/// Observe one frame on the process-wide recorder. A no-op unless
/// `CLI_SESSIONS_RECORD_ANOMALIES` is set, so normal runs pay nothing.
pub fn observe(
    window_id: u64,
    ts: u64,
    title: &str,
    screen: Option<&str>,
    phase: Phase,
    status: Status,
) {
    let slot = RECORDER.get_or_init(|| {
        enabled_dir().map(|_| Mutex::new(AnomalyRecorder::new(RING_CAP, FLAP_SECS)))
    });
    let Some(recorder) = slot else {
        return;
    };
    let Ok(mut recorder) = recorder.lock() else {
        return;
    };
    let frame = Frame {
        ts,
        title: title.to_string(),
        screen: screen.map(str::to_string),
        phase,
        status,
    };
    if let Some(anomaly) = recorder.note(window_id, frame) {
        report(&anomaly);
    }
}

fn report(anomaly: &Anomaly) {
    let Some(dir) = enabled_dir() else { return };
    match dump(&dir, anomaly) {
        Ok(path) => eprintln!(
            "[cli-sessions] recorded {} on win{} (dwell {}s) -> {}",
            anomaly.kind,
            anomaly.window_id,
            anomaly.dwell_secs,
            path.display()
        ),
        Err(e) => eprintln!("[cli-sessions] anomaly dump failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ts: u64, status: Status) -> Frame {
        Frame {
            ts,
            title: "x".into(),
            screen: Some(format!("screen at {ts}")),
            phase: Phase::Blocked,
            status,
        }
    }

    #[test]
    fn short_lived_needs_you_is_a_flap() {
        let mut rec = AnomalyRecorder::new(6, 9);
        assert!(rec.note(1, frame(0, Status::Working)).is_none());
        assert!(rec.note(1, frame(3, Status::NeedsYou)).is_none());
        let anomaly = rec
            .note(1, frame(6, Status::Working))
            .expect("a NeedsYou that clears within the window is a flap");
        assert_eq!(anomaly.kind, "needs_you_flap");
        assert_eq!(anomaly.dwell_secs, 3);
        assert!(
            anomaly.frames.iter().any(|f| f.status == Status::NeedsYou),
            "the dumped ring must include the NeedsYou frame for review"
        );
    }

    #[test]
    fn sustained_needs_you_is_not_a_flap() {
        let mut rec = AnomalyRecorder::new(6, 9);
        rec.note(1, frame(0, Status::NeedsYou));
        rec.note(1, frame(30, Status::NeedsYou));
        assert!(
            rec.note(1, frame(60, Status::Working)).is_none(),
            "a real prompt held for a while before resolving is not a flap"
        );
    }

    #[test]
    fn needs_you_resolving_after_window_is_not_a_flap() {
        let mut rec = AnomalyRecorder::new(6, 9);
        rec.note(1, frame(0, Status::NeedsYou));
        assert!(
            rec.note(1, frame(20, Status::YourTurn)).is_none(),
            "leaving NeedsYou after the flap window does not record"
        );
    }

    #[test]
    fn ring_is_bounded() {
        let mut rec = AnomalyRecorder::new(3, 9);
        for ts in 0..10 {
            rec.note(1, frame(ts, Status::Working));
        }
        rec.note(1, frame(10, Status::NeedsYou));
        let anomaly = rec.note(1, frame(12, Status::Working)).unwrap();
        assert!(
            anomaly.frames.len() <= 3,
            "ring keeps only the most recent frames, got {}",
            anomaly.frames.len()
        );
    }
}
