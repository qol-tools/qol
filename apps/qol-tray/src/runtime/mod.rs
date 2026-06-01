mod channels;
mod poller;
mod publisher;
mod server;
mod state;
#[doc(hidden)]
pub mod testing;

use std::time::Duration;

pub use publisher::publish;
pub use server::RuntimeServer;

pub(crate) trait Channel: Send {
    fn poll(&mut self) -> bool;
    fn min_interval(&self) -> Duration;
}
