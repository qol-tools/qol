use qol_peers::admin::{PointzRequest, Request, Response};
use qol_runtime::PlatformStateClient;

pub(crate) fn begin() {
    match request(PointzRequest::BeginPairing {}) {
        Some(Response::PointzPairing { pairing }) if pairing.open => {
            log::info!(
                "PointZ pairing open for {} seconds",
                pairing.seconds_remaining
            );
        }
        Some(Response::Error { error }) => log::warn!("PointZ pairing refused: {error}"),
        _ => log::warn!("PointZ pairing could not be opened by qol-tray"),
    }
}

pub(crate) fn status_json() -> serde_json::Value {
    let pairing = status()
        .map(|status| status.pairing)
        .unwrap_or(qol_peers::pointz::PointzPairing::CLOSED);
    serde_json::json!({
        "pairing_open": pairing.open,
        "pin": pairing.code,
        "seconds_remaining": pairing.seconds_remaining,
    })
}

fn request(request: PointzRequest) -> Option<Response> {
    PlatformStateClient::from_env()
        .peer_admin(Request::Pointz { request })
        .ok()
}

pub(crate) fn status() -> Option<qol_peers::admin::PointzStatus> {
    match request(PointzRequest::Status {}) {
        Some(Response::PointzStatus { status }) => Some(status),
        _ => None,
    }
}
