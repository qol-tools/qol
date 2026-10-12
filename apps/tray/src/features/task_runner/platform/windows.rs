use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use qol_platform::native::wide::from_code_page;
use windows_sys::Win32::Globalization::CP_OEMCP;

use super::TaskRunnerPlatform;

pub(in crate::features::task_runner) struct Platform;

impl TaskRunnerPlatform for Platform {
    fn shell_command(&self, script: &str) -> Result<Command, String> {
        let mut command = qol_process::system_command("cmd.exe");
        command
            .args(["/D", "/V:OFF", "/S", "/C"])
            .raw_arg(format!("\"{script}\""));
        Ok(command)
    }

    fn render_command(
        &self,
        template: &str,
        params: &HashMap<String, String>,
    ) -> Result<String, String> {
        super::windows_quote::render(template, params)
    }

    fn decode_output(&self, bytes: &[u8]) -> String {
        super::windows_output::decode(bytes, |bytes| from_code_page(bytes, CP_OEMCP))
    }

    fn guardian_executable(&self) -> std::io::Result<PathBuf> {
        std::env::current_exe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_that_is_not_utf8_decodes_with_the_oem_code_page() {
        let bytes = [b'n', 0x81, 0x94];
        let expected = from_code_page(&bytes, CP_OEMCP).unwrap();
        assert_eq!(Platform.decode_output(&bytes), expected);
        assert_eq!(Platform.decode_output("ünicöde".as_bytes()), "ünicöde");
    }
}
