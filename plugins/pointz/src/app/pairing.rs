use qol_peers::admin::{Error, PointzRequest, Request, Response};
use qol_runtime::PlatformStateClient;

pub(crate) fn begin() -> bool {
    let mut answer = request(PointzRequest::BeginPairing {});
    if matches!(
        answer,
        Some(Response::Error {
            error: Error::PointzUnavailable
        })
    ) && matches!(
        request(PointzRequest::Serve { on: true }),
        Some(Response::PointzStatus { .. })
    ) {
        answer = request(PointzRequest::BeginPairing {});
    }
    match answer {
        Some(Response::PointzPairing { pairing }) if pairing.open => {
            log::info!(
                "PointZ pairing open for {} seconds",
                pairing.seconds_remaining
            );
            true
        }
        Some(Response::Error { error }) => {
            log::warn!("PointZ pairing refused: {error}");
            false
        }
        _ => {
            log::warn!("PointZ pairing could not be opened by qol-tray");
            false
        }
    }
}

pub(crate) fn request(request: PointzRequest) -> Option<Response> {
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
