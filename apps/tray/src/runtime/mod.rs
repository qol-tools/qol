mod channels;
mod poller;
mod publisher;
mod server;
mod state;
#[doc(hidden)]
pub mod testing;

use std::time::Duration;

pub(crate) use publisher::events;
pub use publisher::install_events;
pub use publisher::publish;
pub(crate) use server::push_status::PluginStatusRegistry;
#[cfg(test)]
pub(crate) use server::state_store::SharedState;
pub use server::{RuntimeServer, StateSocketStatus};

pub(crate) trait Channel: Send {
    fn poll(&mut self) -> bool;
    fn min_interval(&self) -> Duration;
}
