use anyhow::Result;
use std::time::Duration;

use super::super::serial::SerialClient;
use super::GuestOs;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(180);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(15);
const PROMPT: &str = ":~#";

pub(crate) struct DebianNocloud;

impl GuestOs for DebianNocloud {
    fn ensure_root_shell(&self, serial: &mut SerialClient) -> Result<()> {
        serial.send_line("")?;
        let (marker, _) = serial.wait_for_any(&["login:", PROMPT], LOGIN_TIMEOUT)?;
        if marker == 0 {
            serial.send_line("root")?;
            serial.wait_for(PROMPT, PROMPT_TIMEOUT)?;
        }
        Ok(())
    }
}
