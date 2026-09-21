mod backends;

use anyhow::Result;

use crate::bluetooth::normalize_address;
use backends::pulse_streams;

pub const RECLAIM_SUPPORTED: bool = true;

pub fn reclaim_output(address: &str) -> Result<()> {
    let address = normalize_address(address)?;
    let sink = match pulse_streams::bluetooth_sink(&address) {
        Ok(sink) => sink,
        Err(error) => {
            qol_runtime::probe!(
                "BLUETOOTH_AUDIO_CLAIM",
                "event=reclaim trigger=manual address={address} outcome=failed reason=no_output"
            );
            return Err(error);
        }
    };
    match pulse_streams::suspend_resume(&sink) {
        Ok(()) => {
            qol_runtime::probe!(
                "BLUETOOTH_AUDIO_CLAIM",
                "event=reclaim trigger=manual sink={sink} outcome=ok"
            );
            Ok(())
        }
        Err(error) => {
            qol_runtime::probe!(
                "BLUETOOTH_AUDIO_CLAIM",
                "event=reclaim trigger=manual sink={sink} outcome=failed"
            );
            Err(error)
        }
    }
}
