pub mod platform;

use anyhow::Result;

pub trait ThemeStrategy {
    fn apply(&self) -> Result<()>;
    fn revert(&self) -> Result<()>;
}
