pub mod bridge;
pub mod cli;
mod error;
pub mod kitty;
mod model;
mod service;
mod spawn;

pub use error::{IdentityError, TerminalError};
pub use model::{
    BackendId, DeliveryMode, SessionBinding, SessionCapabilities, SessionFacts, SessionId,
    TerminalSnapshot,
};
pub use service::{
    ScreenReader, SessionCloser, SessionFocus, SessionInventory, TerminalBackend,
    TerminalSessionService, TextInput,
};
pub use spawn::{SessionSpawner, SpawnIdentity, SpawnKey, SpawnRequest, SpawnSurface};
