pub mod bridge;
pub mod cli;
mod error;
pub mod kitty;
pub mod marker;
mod model;
mod service;
mod spawn;

pub use error::{IdentityError, TerminalError};
pub use model::{
    BackendId, DeliveryMode, SessionBinding, SessionCapabilities, SessionFacts, SessionId,
    TerminalSnapshot,
};
pub use service::{
    screen_contains_ignoring_whitespace, ScreenReader, SessionCloser, SessionFocus,
    SessionInventory, TerminalBackend, TerminalSessionService, TextInput, WaitOutcome,
};
pub use spawn::{SessionSpawner, SpawnIdentity, SpawnKey, SpawnRequest, SpawnSurface};
