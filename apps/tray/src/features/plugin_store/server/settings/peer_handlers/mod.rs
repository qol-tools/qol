use axum::{
    body::to_bytes,
    extract::{FromRef, Request as HttpRequest, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use qol_peers::admin::Request;
use qol_runtime::{local_ipc::MAX_MESSAGE_BYTES, PlatformStateClient};
use serde::de::DeserializeOwned;
use std::sync::{Arc, Mutex};

use super::super::types::AppState;
use crate::features::linked_computers::settings::{catalog_operations, invitation_info, Failure};
use crate::plugins::{operation_catalog::CatalogCapture, PluginManager};

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(in crate::features::plugin_store::server) struct CatalogState(
    pub(in crate::features::plugin_store::server) Arc<Mutex<PluginManager>>,
);

impl FromRef<AppState> for CatalogState {
    fn from_ref(state: &AppState) -> Self {
        Self(state.plugin_manager.clone())
    }
}

pub(in crate::features::plugin_store::server) fn routes<S>() -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
    CatalogState: FromRef<S>,
{
    axum::Router::new()
        .route("/peers/admin", axum::routing::post(admin))
        .route("/peers/invitation", axum::routing::post(invitation))
        .route("/peers/catalog", axum::routing::get(catalog))
}

async fn catalog(State(CatalogState(manager)): State<CatalogState>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let capture = {
            let guard = manager
                .lock()
                .map_err(|_| Failure::Transport("Operation catalog unavailable".into()))?;
            CatalogCapture::capture(&guard)
        };
        let resolved = capture
            .try_resolve()
            .map_err(|_| Failure::Transport("Operation catalog unavailable".into()))?;
        Ok::<_, Failure>(catalog_operations(&resolved))
    })
    .await;
    match result {
        Ok(Ok(operations)) => no_store(Json(operations).into_response()),
        Ok(Err(error)) => failure(error),
        Err(_) => failure(Failure::Transport("Operation catalog unavailable".into())),
    }
}

pub(super) async fn admin(request: HttpRequest) -> Response {
    admin_at(
        request,
        PlatformStateClient::new(crate::dev_generation::state_socket_path()),
    )
    .await
}

async fn admin_at(request: HttpRequest, client: PlatformStateClient) -> Response {
    let request: Request = match parse(request).await {
        Ok(request) => request,
        Err(error) => return no_store(error.into_response()),
    };
    let mutation = request.is_mutation();
    let result = tokio::task::spawn_blocking(move || client.peer_admin(request)).await;
    match result {
        Ok(Ok(response)) => no_store(Json(response).into_response()),
        Ok(Err(error)) => failure(error.into()),
        Err(_) if mutation => failure(Failure::OutcomeUnknown),
        Err(_) => failure(Failure::Transport("Local runtime unavailable".into())),
    }
}

pub(super) async fn invitation(request: HttpRequest) -> Response {
    let document = match parse::<qol_peers::enrollment::ExportedInvitation>(request).await {
        Ok(document) => document,
        Err(error) => return no_store(error.into_response()),
    };
    match invitation_info(&document) {
        Ok(info) => no_store(Json(info).into_response()),
        Err(error) => failure(Failure::Authority(error)),
    }
}

async fn parse<T: DeserializeOwned>(request: HttpRequest) -> Result<T, (StatusCode, &'static str)> {
    let body = to_bytes(request.into_body(), MAX_MESSAGE_BYTES)
        .await
        .map_err(|_| (StatusCode::PAYLOAD_TOO_LARGE, "Peer request too large"))?;
    serde_json::from_slice(&body).map_err(|_| (StatusCode::BAD_REQUEST, "Invalid peer request"))
}

fn failure(error: Failure) -> Response {
    no_store((StatusCode::BAD_GATEWAY, Json(error)).into_response())
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
