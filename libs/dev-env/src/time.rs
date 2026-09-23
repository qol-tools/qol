use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

pub fn unix_millis() -> Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before UNIX_EPOCH")?
        .as_millis();
    u64::try_from(millis).context("timestamp does not fit in u64")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_timestamp_fits_the_shared_report_type() {
        assert!(unix_millis().unwrap() > 0);
    }
}
