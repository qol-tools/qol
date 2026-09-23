use super::super::Platform;
use anyhow::Result;

pub(super) fn current() -> Result<Platform> {
    Ok(Platform::Macos)
}
