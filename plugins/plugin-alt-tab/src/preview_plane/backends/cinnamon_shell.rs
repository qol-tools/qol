use crate::preview_plane::PreviewPlanePayload;
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const BACKEND: &str = "cinnamon_shell";
const DEST: &str = "org.Cinnamon";
const OBJECT_PATH: &str = "/org/qol/AltTabPreviewPlane";
const PING_METHOD: &str = "org.qol.AltTabPreviewPlane.Ping";
const SHOW_METHOD: &str = "org.qol.AltTabPreviewPlane.Show";
const HIDE_METHOD: &str = "org.qol.AltTabPreviewPlane.Hide";
const AVAILABILITY_TTL: Duration = Duration::from_secs(2);
const DBUS_TIMEOUT_SECONDS: &str = "1";

#[derive(Debug, Clone, Copy)]
struct Availability {
    checked_at: Instant,
    available: bool,
}

enum PlaneCommand {
    Show {
        show_id: String,
        item_count: usize,
        payload_json: String,
    },
    Hide {
        reason: &'static str,
    },
}

fn availability_cache() -> &'static Mutex<Option<Availability>> {
    static CACHE: OnceLock<Mutex<Option<Availability>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn command_queue() -> &'static Sender<PlaneCommand> {
    static QUEUE: OnceLock<Sender<PlaneCommand>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(command) = rx.recv() {
                match command {
                    PlaneCommand::Show {
                        show_id,
                        item_count,
                        payload_json,
                    } => run_show_command(show_id, item_count, payload_json),
                    PlaneCommand::Hide { reason } => run_hide_command(reason),
                }
            }
        });
        tx
    })
}

pub(crate) fn disabled_reason() -> Option<&'static str> {
    if let Ok(value) = std::env::var("QOL_ALT_TAB_CINNAMON_PREVIEW_PLANE") {
        if value == "1" || value.eq_ignore_ascii_case("true") {
            return None;
        }
        if value == "0" || value.eq_ignore_ascii_case("false") {
            return Some("env_disabled");
        }
    }
    let is_cinnamon = std::env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| std::env::var("DESKTOP_SESSION"))
        .map(|desktop| desktop.to_ascii_lowercase().contains("cinnamon"))
        .unwrap_or(false);
    (!is_cinnamon).then_some("session_not_cinnamon")
}

pub(crate) fn live_preview_replacement() -> Option<&'static str> {
    if disabled_reason().is_some() {
        return None;
    }
    ping_available_cached().then_some(BACKEND)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AvailabilityDecision {
    Fresh(bool),
    Stale(bool),
    Cold,
}

fn availability_decision(cache: Option<Availability>, now: Instant) -> AvailabilityDecision {
    match cache {
        None => AvailabilityDecision::Cold,
        Some(entry) if now.duration_since(entry.checked_at) < AVAILABILITY_TTL => {
            AvailabilityDecision::Fresh(entry.available)
        }
        Some(entry) => AvailabilityDecision::Stale(entry.available),
    }
}

fn ping_available_cached() -> bool {
    let now = Instant::now();
    let Ok(mut cache) = availability_cache().lock() else {
        return false;
    };
    match availability_decision(*cache, now) {
        AvailabilityDecision::Fresh(available) => available,
        AvailabilityDecision::Stale(available) => {
            spawn_availability_refresh();
            available
        }
        AvailabilityDecision::Cold => {
            let available = ping_available();
            *cache = Some(Availability {
                checked_at: Instant::now(),
                available,
            });
            available
        }
    }
}

fn spawn_availability_refresh() {
    static REFRESHING: AtomicBool = AtomicBool::new(false);
    if REFRESHING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| {
        let available = ping_available();
        if let Ok(mut cache) = availability_cache().lock() {
            *cache = Some(Availability {
                checked_at: Instant::now(),
                available,
            });
        }
        REFRESHING.store(false, Ordering::Release);
    });
}

fn ping_available() -> bool {
    ProcessCommand::new("gdbus")
        .args([
            "call",
            "--session",
            "--timeout",
            DBUS_TIMEOUT_SECONDS,
            "--dest",
            DEST,
            "--object-path",
            OBJECT_PATH,
            "--method",
            PING_METHOD,
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).contains("\"ok\":true"))
        .unwrap_or(false)
}

pub(crate) fn show_async(payload: PreviewPlanePayload) {
    let show_id = payload.show_id.clone();
    let item_count = payload.items.len();
    let Ok(payload_json) = serde_json::to_string(&payload) else {
        qol_runtime::probe!(
            "PREVIEW_PLANE_SHOW",
            "backend=cinnamon_shell show_id={show_id} outcome=skipped reason=json items={item_count}"
        );
        return;
    };

    qol_runtime::probe!(
        "PREVIEW_PLANE_SHOW",
        "backend=cinnamon_shell show_id={show_id} outcome=queued items={item_count}"
    );
    if command_queue()
        .send(PlaneCommand::Show {
            show_id: show_id.clone(),
            item_count,
            payload_json,
        })
        .is_err()
    {
        qol_runtime::probe!(
            "PREVIEW_PLANE_SHOW",
            "backend=cinnamon_shell show_id={show_id} outcome=error reason=queue_closed items={item_count}"
        );
    }
}

pub(crate) fn hide_async(reason: &'static str) {
    qol_runtime::probe!(
        "PREVIEW_PLANE_HIDE",
        "backend=cinnamon_shell reason={reason} outcome=queued"
    );
    if command_queue().send(PlaneCommand::Hide { reason }).is_err() {
        qol_runtime::probe!(
            "PREVIEW_PLANE_HIDE",
            "backend=cinnamon_shell reason={reason} outcome=error reason=queue_closed"
        );
    }
}

fn run_show_command(show_id: String, item_count: usize, payload_json: String) {
    let started = Instant::now();
    let result = ProcessCommand::new("gdbus")
        .args([
            "call",
            "--session",
            "--timeout",
            DBUS_TIMEOUT_SECONDS,
            "--dest",
            DEST,
            "--object-path",
            OBJECT_PATH,
            "--method",
            SHOW_METHOD,
        ])
        .arg(payload_json)
        .output();
    probe_show_result(&show_id, item_count, started, result);
}

fn run_hide_command(reason: &'static str) {
    let started = Instant::now();
    let result = ProcessCommand::new("gdbus")
        .args([
            "call",
            "--session",
            "--timeout",
            DBUS_TIMEOUT_SECONDS,
            "--dest",
            DEST,
            "--object-path",
            OBJECT_PATH,
            "--method",
            HIDE_METHOD,
        ])
        .output();
    probe_hide_result(reason, started, result);
}

fn probe_show_result(
    show_id: &str,
    item_count: usize,
    started: Instant,
    result: std::io::Result<std::process::Output>,
) {
    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            qol_runtime::probe!(
                "PREVIEW_PLANE_SHOW",
                "backend=cinnamon_shell show_id={show_id} outcome=ok items={item_count} elapsed={elapsed_ms}ms result=\"{}\"",
                trim_for_probe(&stdout)
            );
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            qol_runtime::probe!(
                "PREVIEW_PLANE_SHOW",
                "backend=cinnamon_shell show_id={show_id} outcome=error items={item_count} status={} elapsed={}ms stderr=\"{}\"",
                output.status,
                elapsed_ms,
                trim_for_probe(&stderr)
            );
        }
        Err(error) => {
            qol_runtime::probe!(
                "PREVIEW_PLANE_SHOW",
                "backend=cinnamon_shell show_id={show_id} outcome=error items={item_count} elapsed={}ms error=\"{}\"",
                elapsed_ms,
                error
            );
        }
    }
}

fn probe_hide_result(
    reason: &'static str,
    started: Instant,
    result: std::io::Result<std::process::Output>,
) {
    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Ok(output) if output.status.success() => {
            qol_runtime::probe!(
                "PREVIEW_PLANE_HIDE",
                "backend=cinnamon_shell reason={reason} outcome=ok elapsed={elapsed_ms}ms"
            );
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            qol_runtime::probe!(
                "PREVIEW_PLANE_HIDE",
                "backend=cinnamon_shell reason={reason} outcome=error status={} elapsed={}ms stderr=\"{}\"",
                output.status,
                elapsed_ms,
                trim_for_probe(&stderr)
            );
        }
        Err(error) => {
            qol_runtime::probe!(
                "PREVIEW_PLANE_HIDE",
                "backend=cinnamon_shell reason={reason} outcome=error elapsed={}ms error=\"{}\"",
                elapsed_ms,
                error
            );
        }
    }
}

fn trim_for_probe(s: &str) -> String {
    let one_line = s.replace(['\n', '\r'], " ");
    one_line.chars().take(220).collect()
}

#[cfg(test)]
mod tests {
    use super::{availability_decision, Availability, AvailabilityDecision, AVAILABILITY_TTL};
    use std::time::{Duration, Instant};

    #[test]
    fn availability_decision_classifies_cache_state() {
        let now = Instant::now();
        let fresh = now - AVAILABILITY_TTL / 2;
        let stale = now - (AVAILABILITY_TTL + Duration::from_millis(1));
        let cases = [
            (None, AvailabilityDecision::Cold),
            (
                Some(Availability {
                    checked_at: fresh,
                    available: true,
                }),
                AvailabilityDecision::Fresh(true),
            ),
            (
                Some(Availability {
                    checked_at: fresh,
                    available: false,
                }),
                AvailabilityDecision::Fresh(false),
            ),
            (
                Some(Availability {
                    checked_at: stale,
                    available: true,
                }),
                AvailabilityDecision::Stale(true),
            ),
        ];
        for (cache, expected) in cases {
            assert_eq!(
                availability_decision(cache, now),
                expected,
                "cache: {cache:?}"
            );
        }
    }
}
