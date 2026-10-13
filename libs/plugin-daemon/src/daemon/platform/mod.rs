#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
use fallback as active;
#[cfg(unix)]
use unix as active;

pub(super) use active::{fallback_socket_dir, inherited_listener};
pub use active::{inherited_port_fd, inherited_primary_port_fd, restore_cloexec};
