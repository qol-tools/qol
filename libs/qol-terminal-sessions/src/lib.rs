pub mod cli;
mod error;
pub mod kitty;
mod model;
mod service;

pub use error::{IdentityError, TerminalError};
pub use model::{
    BackendId, DeliveryMode, SessionBinding, SessionCapabilities, SessionFacts, SessionId,
};
pub use service::{
    ScreenReader, SessionFocus, SessionInventory, TerminalBackend, TerminalSessionService,
    TextInput,
};
