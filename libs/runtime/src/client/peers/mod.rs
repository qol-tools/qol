#[cfg(test)]
mod tests;

use std::fmt;
use std::io;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

use qol_peers::admin::{Request, Response};

use super::{platform, PlatformStateClient};
use crate::local_ipc::MAX_MESSAGE_BYTES;
use crate::protocol::RuntimeRequest;

const ADMIN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerAdminClientError {
    RequestTooLarge,
    Encode,
    UnsupportedPlatform,
    Unavailable,
    Unauthorized,
    Timeout,
    Disconnected,
    InvalidReply,
    OutcomeUnknown,
}

impl fmt::Display for PeerAdminClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::RequestTooLarge => "peer administration request exceeds the local message boundary",
            Self::Encode => "peer administration request could not be encoded",
            Self::UnsupportedPlatform => "local runtime transport is unsupported on this platform",
            Self::Unavailable => "local runtime is unavailable",
            Self::Unauthorized => "local runtime peer credentials were rejected",
            Self::Timeout => "peer administration reply timed out",
            Self::Disconnected => "peer administration connection closed",
            Self::InvalidReply => "peer administration reply is invalid or too large",
            Self::OutcomeUnknown => "peer administration mutation outcome is unknown; inspect status before deciding what to do next",
        })
    }
}

impl std::error::Error for PeerAdminClientError {}

impl PlatformStateClient {
    pub fn peer_admin(&self, request: Request) -> Result<Response, PeerAdminClientError> {
        let mutation = request.is_mutation();
        let payload = encode(request)?;
        let connection = platform::connect(&self.socket_path).map_err(connection_error)?;
        exchange(connection, &payload, mutation, ADMIN_TIMEOUT)
    }
}

fn encode(request: Request) -> Result<Zeroizing<Vec<u8>>, PeerAdminClientError> {
    use crate::local_ipc::{encode_secret_json, SecretMessageError};
    encode_secret_json(&RuntimeRequest::PeerAdmin { request }).map_err(|error| match error {
        SecretMessageError::TooLarge => PeerAdminClientError::RequestTooLarge,
        SecretMessageError::Encode => PeerAdminClientError::Encode,
    })
}

fn exchange_value<T: serde::de::DeserializeOwned>(
    mut connection: Box<dyn platform::Connection>,
    payload: &[u8],
    mutation: bool,
    timeout: Duration,
) -> Result<T, PeerAdminClientError> {
    connection
        .set_write_timeout(Some(timeout))
        .map_err(connection_error)?;
    connection
        .set_read_timeout(Some(timeout))
        .map_err(connection_error)?;
    let deadline = Instant::now() + timeout;
    write_request(connection.as_mut(), payload, deadline)
        .map_err(|error| delivered_error(mutation, error))?;
    let reply = read_reply(connection.as_mut(), deadline)
        .map_err(|error| delivered_error(mutation, error))?;
    serde_json::from_slice(&reply)
        .map_err(|_| delivered_error(mutation, PeerAdminClientError::InvalidReply))
}

fn write_request(
    connection: &mut dyn platform::Connection,
    mut payload: &[u8],
    deadline: Instant,
) -> Result<(), PeerAdminClientError> {
    while !payload.is_empty() {
        connection
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(io_error)?;
        let count = connection.write(payload).map_err(io_error)?;
        if count == 0 {
            return Err(PeerAdminClientError::Disconnected);
        }
        payload = &payload[count..];
    }
    Ok(())
}

fn remaining(deadline: Instant) -> Result<Duration, PeerAdminClientError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(PeerAdminClientError::Timeout)
}

fn read_reply(
    connection: &mut dyn platform::Connection,
    deadline: Instant,
) -> Result<Zeroizing<Vec<u8>>, PeerAdminClientError> {
    let mut reply = Zeroizing::new(Vec::with_capacity(MAX_MESSAGE_BYTES));
    loop {
        let remaining = remaining(deadline)?;
        // macOS refuses a new timeout once the server has replied and closed;
        // the reply is still buffered, so read it under the previous timeout
        if let Err(error) = connection.set_read_timeout(Some(remaining)) {
            if error.kind() != io::ErrorKind::InvalidInput {
                return Err(io_error(error));
            }
        }
        let mut chunk = Zeroizing::new([0; 1024]);
        let count = connection.read(&mut chunk[..]).map_err(io_error)?;
        if count == 0 {
            return Err(PeerAdminClientError::Disconnected);
        }
        if reply.len() + count > MAX_MESSAGE_BYTES {
            return Err(PeerAdminClientError::InvalidReply);
        }
        reply.extend_from_slice(&chunk[..count]);
        if let Some(index) = reply.iter().position(|byte| *byte == b'\n') {
            if index + 1 != reply.len() {
                return Err(PeerAdminClientError::InvalidReply);
            }
            return Ok(reply);
        }
    }
}

fn delivered_error(mutation: bool, error: PeerAdminClientError) -> PeerAdminClientError {
    if mutation {
        PeerAdminClientError::OutcomeUnknown
    } else {
        error
    }
}

fn connection_error(error: io::Error) -> PeerAdminClientError {
    match error.kind() {
        io::ErrorKind::Unsupported => PeerAdminClientError::UnsupportedPlatform,
        io::ErrorKind::PermissionDenied => PeerAdminClientError::Unauthorized,
        _ => PeerAdminClientError::Unavailable,
    }
}

fn io_error(error: io::Error) -> PeerAdminClientError {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => PeerAdminClientError::Timeout,
        _ => PeerAdminClientError::Disconnected,
    }
}

impl PlatformStateClient {
    pub fn peer_operation(
        &self,
        request: qol_peers::operations::Request,
    ) -> Result<qol_peers::operations::Response, PeerAdminClientError> {
        let payload =
            crate::local_ipc::encode_secret_json(&RuntimeRequest::PeerOperation { request })
                .map_err(|_| PeerAdminClientError::RequestTooLarge)?;
        let connection = platform::connect(&self.socket_path).map_err(connection_error)?;
        exchange_value(connection, &payload, true, Duration::from_secs(12))
    }
}

fn exchange(
    connection: Box<dyn platform::Connection>,
    payload: &[u8],
    mutation: bool,
    timeout: Duration,
) -> Result<Response, PeerAdminClientError> {
    exchange_value(connection, payload, mutation, timeout)
}
