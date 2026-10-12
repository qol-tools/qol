use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_OEMCP};

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
        super::windows_output::decode(bytes, |bytes| decode_code_page(bytes, CP_OEMCP))
    }

    fn guardian_executable(&self) -> std::io::Result<PathBuf> {
        std::env::current_exe()
    }
}

fn decode_code_page(bytes: &[u8], code_page: u32) -> Option<String> {
    let len = i32::try_from(bytes.len()).ok()?;
    let wide_len =
        unsafe { MultiByteToWideChar(code_page, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0) };
    if wide_len <= 0 {
        return None;
    }
    let mut wide = vec![0u16; wide_len as usize];
    let written = unsafe {
        MultiByteToWideChar(
            code_page,
            0,
            bytes.as_ptr(),
            len,
            wide.as_mut_ptr(),
            wide_len,
        )
    };
    if written <= 0 {
        return None;
    }
    wide.truncate(written as usize);
    Some(String::from_utf16_lossy(&wide))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oem_output_decodes_through_the_given_code_page() {
        assert_eq!(decode_code_page(&[0x81, 0x94], 850).as_deref(), Some("üö"));
    }

    #[test]
    fn output_that_is_not_utf8_decodes_with_the_oem_code_page() {
        let bytes = [b'n', 0x81, 0x94];
        let expected = decode_code_page(&bytes, CP_OEMCP).unwrap();
        assert_eq!(Platform.decode_output(&bytes), expected);
        assert_eq!(Platform.decode_output("ünicöde".as_bytes()), "ünicöde");
    }
}
