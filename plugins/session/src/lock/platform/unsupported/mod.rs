use anyhow::{bail, Result};

use crate::lock::Availability;

pub fn lock_screen() -> Result<()> {
    bail!(
        "locking the screen is not supported on {}",
        qol_platform::os::current_name()
    );
}

pub fn availability() -> Availability {
    Availability::Missing {
        detail: format!(
            "Locking the screen is not supported on {}.",
            qol_platform::os::current_name()
        ),
        fix: "Run Session on Linux, macOS or Windows.".into(),
    }
}
