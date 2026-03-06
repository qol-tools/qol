mod channels;
mod poller;
mod server;
mod state;

use std::time::Duration;

pub use server::RuntimeServer;

pub(crate) trait Channel: Send {
    fn poll(&mut self) -> bool;
    fn min_interval(&self) -> Duration;
}
