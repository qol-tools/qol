mod io;
mod requests;

use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;

use super::shared::SharedState;
use crate::paths::STATE_SOCKET_PATH;
use io::{prepare_stream, read_request};
use requests::handle_request;

pub(super) fn run(shared: Arc<SharedState>) {
    let _ = std::fs::remove_file(STATE_SOCKET_PATH);

    let Some(listener) = bind_listener() else {
        return;
    };

    log::info!("Runtime socket listening on {}", STATE_SOCKET_PATH);

    for stream in listener.incoming() {
        spawn_connection(stream, Arc::clone(&shared));
    }
}

fn bind_listener() -> Option<UnixListener> {
    match UnixListener::bind(STATE_SOCKET_PATH) {
        Ok(listener) => Some(listener),
        Err(error) => {
            log::error!(
                "Failed to bind runtime socket at {}: {}",
                STATE_SOCKET_PATH,
                error
            );
            None
        }
    }
}

fn handle_connection(stream: UnixStream, shared: &SharedState) {
    let Some((mut reader, mut writer)) = prepare_stream(stream) else {
        return;
    };

    let Some(request) = read_request(&mut reader) else {
        return;
    };

    handle_request(&request, &mut writer, shared);
}

fn spawn_connection(stream: std::io::Result<UnixStream>, shared: Arc<SharedState>) {
    let Ok(stream) = stream else {
        return;
    };

    let _ = std::thread::Builder::new()
        .name("runtime-conn".into())
        .spawn(move || handle_connection(stream, &shared));
}
