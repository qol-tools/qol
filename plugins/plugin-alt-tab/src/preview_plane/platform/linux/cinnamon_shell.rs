use crate::preview_plane::PreviewPlanePayload;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use zbus::zvariant::DynamicType;

const BACKEND: &str = "cinnamon_shell";
const DEST: &str = "org.Cinnamon";
const OBJECT_PATH: &str = "/org/qol/AltTabPreviewPlane";
const INTERFACE: &str = "org.qol.AltTabPreviewPlane";
const PING_METHOD: &str = "Ping";
const SHOW_METHOD: &str = "Show";
const HIDE_METHOD: &str = "Hide";
const AVAILABILITY_TTL: Duration = Duration::from_secs(2);
const DBUS_TIMEOUT: Duration = Duration::from_secs(1);

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

#[derive(Deserialize)]
struct PlaneResponse {
    ok: bool,
    code: Option<String>,
    detail: Option<String>,
}

fn availability_cache() -> &'static Mutex<Option<Availability>> {
    static CACHE: OnceLock<Mutex<Option<Availability>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn last_payload_cache() -> &'static Mutex<Option<String>> {
    static CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn should_send_show(payload_json: &str, last: Option<&str>) -> bool {
    last != Some(payload_json)
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
    call_plane_method(PING_METHOD, &()).is_ok()
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

    if let Ok(mut last) = last_payload_cache().lock() {
        if !should_send_show(&payload_json, last.as_deref()) {
            qol_runtime::probe!(
                "PREVIEW_PLANE_SHOW",
                "backend=cinnamon_shell show_id={show_id} outcome=skipped reason=unchanged items={item_count}"
            );
            return;
        }
        *last = Some(payload_json.clone());
    }

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
    if let Ok(mut last) = last_payload_cache().lock() {
        *last = None;
    }
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
    let result = call_plane_method(SHOW_METHOD, &show_arguments(&payload_json));
    probe_show_result(&show_id, item_count, started, result);
}

fn run_hide_command(reason: &'static str) {
    let started = Instant::now();
    let result = call_plane_method(HIDE_METHOD, &());
    probe_hide_result(reason, started, result);
}

fn show_arguments(payload_json: &str) -> (&str,) {
    (payload_json,)
}

fn call_plane_method<B>(method: &str, body: &B) -> Result<String, String>
where
    B: serde::Serialize + DynamicType,
{
    let connection = zbus::blocking::connection::Builder::session()
        .map_err(|error| format!("session connection: {error}"))?
        .method_timeout(DBUS_TIMEOUT)
        .build()
        .map_err(|error| format!("session connection: {error}"))?;
    let proxy = zbus::blocking::Proxy::new(&connection, DEST, OBJECT_PATH, INTERFACE)
        .map_err(|error| format!("preview plane proxy: {error}"))?;
    let response: String = proxy
        .call(method, body)
        .map_err(|error| format!("{method} call: {error}"))?;
    validate_plane_response(&response)?;
    Ok(response)
}

fn validate_plane_response(response: &str) -> Result<(), String> {
    let parsed: PlaneResponse =
        serde_json::from_str(response).map_err(|error| format!("invalid response: {error}"))?;
    if parsed.ok {
        return Ok(());
    }
    let code = parsed.code.as_deref().unwrap_or("rejected");
    let detail = parsed.detail.as_deref().unwrap_or("no detail");
    Err(format!("{code}: {detail}"))
}

fn probe_show_result(
    show_id: &str,
    item_count: usize,
    started: Instant,
    result: Result<String, String>,
) {
    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Ok(response) => {
            qol_runtime::probe!(
                "PREVIEW_PLANE_SHOW",
                "backend=cinnamon_shell show_id={show_id} outcome=ok items={item_count} elapsed={elapsed_ms}ms result=\"{}\"",
                trim_for_probe(&response)
            );
        }
        Err(error) => {
            qol_runtime::probe!(
                "PREVIEW_PLANE_SHOW",
                "backend=cinnamon_shell show_id={show_id} outcome=error items={item_count} elapsed={elapsed_ms}ms error=\"{}\"",
                trim_for_probe(&error)
            );
        }
    }
}

fn probe_hide_result(reason: &'static str, started: Instant, result: Result<String, String>) {
    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Ok(_) => {
            qol_runtime::probe!(
                "PREVIEW_PLANE_HIDE",
                "backend=cinnamon_shell reason={reason} outcome=ok elapsed={elapsed_ms}ms"
            );
        }
        Err(error) => {
            qol_runtime::probe!(
                "PREVIEW_PLANE_HIDE",
                "backend=cinnamon_shell reason={reason} outcome=error elapsed={elapsed_ms}ms error=\"{}\"",
                trim_for_probe(&error)
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
    use super::{
        availability_decision, should_send_show, show_arguments, validate_plane_response,
        Availability, AvailabilityDecision, AVAILABILITY_TTL,
    };
    use std::time::{Duration, Instant};
    use zbus::zvariant::{serialized::Context, to_bytes, LE};

    #[test]
    fn dbus_show_body_preserves_json_escaped_window_titles() {
        let title = "Difference between >> and >\\> operators? Henry's \"notes\"\nNä";
        let payload_json = serde_json::to_string(&serde_json::json!({ "title": title })).unwrap();
        let encoded = to_bytes(Context::new_dbus(LE, 0), &show_arguments(&payload_json)).unwrap();
        let decoded: (&str,) = encoded.deserialize().unwrap().0;
        let decoded_json: serde_json::Value = serde_json::from_str(decoded.0).unwrap();

        assert_eq!(decoded.0, payload_json);
        assert_eq!(decoded_json["title"], title);
    }

    #[test]
    fn plane_response_requires_semantic_success() {
        let cases = [
            ("success", r#"{"ok":true}"#, true),
            (
                "extension rejection",
                r#"{"ok":false,"code":"invalid_json","detail":"bad escape"}"#,
                false,
            ),
            ("malformed response", "not json", false),
        ];
        for (case, response, expected) in cases {
            assert_eq!(
                validate_plane_response(response).is_ok(),
                expected,
                "case: {case}"
            );
        }
    }

    #[test]
    fn should_send_show_skips_only_identical_repeat() {
        let payload_a = "{\"show_id\":\"visible\",\"items\":[{\"wid\":1}]}";
        let payload_b = "{\"show_id\":\"visible\",\"items\":[{\"wid\":2}]}";
        let cases = [
            ("first call, no prior payload", payload_a, None, true),
            ("identical repeat", payload_a, Some(payload_a), false),
            (
                "different payload after previous send",
                payload_b,
                Some(payload_a),
                true,
            ),
            (
                "reverse: previously B, now A",
                payload_a,
                Some(payload_b),
                true,
            ),
            ("empty first call", "", None, true),
            ("empty identical repeat", "", Some(""), false),
        ];
        for (case, payload, last, expected) in cases {
            assert_eq!(should_send_show(payload, last), expected, "case: {case}");
        }
    }

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
