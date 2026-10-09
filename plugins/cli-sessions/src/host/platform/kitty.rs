use std::sync::Arc;

use crate::host::kitty::Kitty;
use crate::host::TerminalHost;

pub(in crate::host) fn system() -> Arc<dyn TerminalHost + Send + Sync> {
    Arc::new(Kitty::default())
}

pub(in crate::host) fn console_probe(_args: &[String]) -> anyhow::Result<String> {
    anyhow::bail!("console probing reads Windows consoles and is unavailable on this platform")
}
