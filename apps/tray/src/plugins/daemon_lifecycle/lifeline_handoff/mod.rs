mod platform;

pub use platform::{adopt_handed_off_fds, prepare_for_exec};
#[cfg_attr(windows, allow(unused_imports))]
pub(crate) use platform::{register, unregister};
