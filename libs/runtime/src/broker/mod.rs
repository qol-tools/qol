//! AF_UNIX broker for inter-plugin dispatch.
//!
//! The broker replaces HTTP-over-loopback with a per-uid Unix-domain
//! socket. Connections are accepted only from the same uid (verified
//! both at the filesystem layer via mode 0600 on the socket file and
//! at the protocol layer via peer credentials). Plugins reach pane
//! fields through a capability-gated pull API and exchange events
//! through a capability-gated publish/subscribe bus on the same
//! socket.
//!
//! See `docs/adr/RUNTIME-1-af-unix-broker-with-peer-cred-auth-and-pull-pane-f.md`.

mod bus;
mod client;
mod field;
mod path;
mod peer_cred;
mod protocol;
mod topic;

pub use crate::pane_field::PaneField;
pub use bus::{EventBus, BUS_QUEUE_CAPACITY};
pub use client::{EventBusClient, EventSubscription};
pub use field::{check_field_access, FieldCheckError};
pub use path::{
    bind_broker_listener, broker_socket_path, broker_socket_path_for_uid, BrokerListener,
    BrokerPathError,
};
pub use peer_cred::{is_same_uid, peer_cred, PeerCred, PeerStream};
pub use protocol::{BrokerEvent, BrokerRequest, SubscribeAck};
pub use topic::{check_topic_access, TopicCheckError};
