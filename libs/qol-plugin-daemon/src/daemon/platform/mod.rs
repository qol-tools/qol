#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
pub use fallback::*;
#[cfg(unix)]
pub use unix::*;
