use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use qol_conventions::operations::{OperationKey, OperationKind};
use qol_conventions::plugin_id::PluginUid;
use qol_peers::admin::{ExpectedAuthority, PageCursor, Request, Response};
use qol_peers::operations::{self, OperationBody, Outcome};
use qol_runtime::PlatformStateClient;
use serde_json::Value;

use super::{Operation, Peer, Remote, RemoteError};

const BLUETOOTH_UID: &str = "c30275fa-eb76-42af-bdce-2e7596976fba";
const OUTCOME_WAIT: Duration = Duration::from_secs(12);
const OUTCOME_POLL: Duration = Duration::from_millis(250);

pub(crate) struct CoreRemote {
    client: PlatformStateClient,
    expected: std::cell::Cell<Option<ExpectedAuthority>>,
}

impl CoreRemote {
    pub(crate) fn from_env() -> Self {
        Self {
            client: PlatformStateClient::from_env(),
            expected: std::cell::Cell::new(None),
        }
    }

    fn admin(&self, request: Request) -> Result<Response> {
        match self.client.peer_admin(request)? {
            Response::Error { error } => bail!("{error}"),
            response => Ok(response),
        }
    }
}

impl Remote for CoreRemote {
    fn peers(&self) -> Result<Vec<Peer>> {
        let Response::Status { status } = self.admin(Request::Status)? else {
            bail!("core returned an unexpected status reply");
        };
        let authority = status
            .authority
            .ok_or_else(|| anyhow!("linked devices are turned off"))?;
        let expected = authority.expected();
        self.expected.set(Some(expected));
        let mut cursor = PageCursor {
            authority_id: expected.authority_id,
            activation_id: expected.activation_id,
            revision: expected.revision,
            offset: 0,
        };
        let mut peers = Vec::new();
        loop {
            let Response::Peers { page } = self.admin(Request::Peers { cursor })? else {
                bail!("core returned an unexpected peer page");
            };
            peers.extend(page.items.into_iter().map(|peer| Peer {
                id: peer.peer_id.to_string(),
                name: peer.name,
            }));
            let Some(next) = page.next else {
                return Ok(peers);
            };
            cursor = next;
        }
    }

    fn call(&self, peer: &Peer, operation: Operation, address: &str) -> Result<Value, RemoteError> {
        let expected = self
            .expected
            .get()
            .ok_or_else(|| RemoteError::Refused("linked devices were not read".into()))?;
        let recipient = peer
            .id
            .parse()
            .map_err(|_| RemoteError::Refused("invalid linked device".into()))?;
        let kind = match operation {
            Operation::State => OperationKind::Query,
            Operation::Release | Operation::Resume => OperationKind::Action,
        };
        let body = serde_json::to_string(&OperationBody {
            version: 1,
            key: OperationKey::new(PluginUid::new(BLUETOOTH_UID), kind, operation.name()),
            arguments: serde_json::json!({ "address": address }).to_string(),
            timeout_ms: operations::MAX_TIMEOUT_MS,
        })
        .map_err(|_| RemoteError::Refused("the request could not be encoded".into()))?;
        let reply = self
            .client
            .peer_operation(operations::Request::Invoke {
                expected,
                peer: recipient,
                body,
            })
            .map_err(|_| RemoteError::Unknown)?;
        let mut status = match reply {
            operations::Response::Status { status } => status,
            operations::Response::Error { error } => {
                return Err(RemoteError::Refused(refusal(error)))
            }
            operations::Response::Unknown { .. } | operations::Response::Requests { .. } => {
                return Err(RemoteError::Unknown)
            }
        };
        let deadline = Instant::now() + OUTCOME_WAIT;
        while !status.outcome.terminal() && Instant::now() < deadline {
            std::thread::sleep(OUTCOME_POLL);
            match self.client.peer_operation(operations::Request::Outcome {
                expected,
                handle: status.handle.clone(),
            }) {
                Ok(operations::Response::Status { status: current }) => status = current,
                _ => return Err(RemoteError::Unknown),
            }
        }
        match status.outcome {
            Outcome::Result { document } => serde_json::from_str(&document)
                .map_err(|_| RemoteError::Refused("the reply was not readable".into())),
            Outcome::Acknowledged => Ok(Value::Null),
            Outcome::Refused { reason } => Err(RemoteError::Refused(refusal(reason))),
            Outcome::CancelledBeforeDispatch => Err(RemoteError::Refused(
                "it was cancelled before it ran".into(),
            )),
            Outcome::HandlerError | Outcome::HandlerFallback => Err(RemoteError::Refused(
                "its Bluetooth plugin reported an error".into(),
            )),
            Outcome::Accepted | Outcome::DispatchStarted | Outcome::Unknown => {
                Err(RemoteError::Unknown)
            }
        }
    }
}

fn refusal(failure: operations::Failure) -> String {
    match failure {
        operations::Failure::GrantRequired => {
            "this computer has not been allowed to do that there".into()
        }
        operations::Failure::Unavailable | operations::Failure::StaleSession => {
            "it is not connected".into()
        }
        operations::Failure::UpdateRequired => "its Bluetooth plugin needs an update".into(),
        other => format!("{other:?}"),
    }
}
