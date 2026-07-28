mod platform;

use anyhow::Result;
use platform::{Platform, SettingsPlatform};

pub(crate) fn open() -> Result<()> {
    Platform.open()
}
