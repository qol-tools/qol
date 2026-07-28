mod platform;

pub use platform::{adopt_handed_off_fds, prepare_for_exec};
pub(crate) use platform::{register, unregister};
