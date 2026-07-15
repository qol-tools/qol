use anyhow::{bail, Context, Result};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

const EXIT_MARKER_COPIES: usize = 5;
static SERIAL_NONCE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct SerialClient<S = TcpStream> {
    stream: S,
    buffer: String,
    marker_nonce: String,
    command_sequence: u64,
}

pub(crate) fn connect(port: u16, timeout: Duration) -> Result<SerialClient> {
    let deadline = Instant::now() + timeout;
    let address = format!("127.0.0.1:{port}");
    loop {
        match TcpStream::connect(&address) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(Duration::from_millis(200)))
                    .context("failed to set serial read timeout")?;
                return Ok(SerialClient {
                    stream,
                    buffer: String::new(),
                    marker_nonce: new_marker_nonce(),
                    command_sequence: 0,
                });
            }
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(error)
                        .with_context(|| format!("serial connect to {address} timed out"));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

impl<S: Read + Write> SerialClient<S> {
    #[cfg(test)]
    fn from_transport(stream: S) -> Self {
        Self {
            stream,
            buffer: String::new(),
            marker_nonce: "test".to_owned(),
            command_sequence: 0,
        }
    }

    #[cfg(test)]
    fn transport(&self) -> &S {
        &self.stream
    }

    pub(crate) fn send_line(&mut self, line: &str) -> Result<()> {
        self.stream
            .write_all(format!("{line}\n").as_bytes())
            .context("failed to write to serial console")
    }

    pub(crate) fn wait_for(&mut self, marker: &str, timeout: Duration) -> Result<String> {
        Ok(self.wait_for_any(&[marker], timeout)?.1)
    }

    pub(crate) fn wait_for_any(
        &mut self,
        markers: &[&str],
        timeout: Duration,
    ) -> Result<(usize, String)> {
        let deadline = Instant::now() + timeout;
        loop {
            let hit = markers
                .iter()
                .enumerate()
                .filter_map(|(index, marker)| {
                    self.buffer
                        .find(marker)
                        .map(|position| (position + marker.len(), index))
                })
                .min();
            if let Some((end, index)) = hit {
                let consumed: String = self.buffer.drain(..end).collect();
                return Ok((index, consumed));
            }
            if Instant::now() >= deadline {
                let mut start = self.buffer.len().saturating_sub(600);
                while !self.buffer.is_char_boundary(start) {
                    start -= 1;
                }
                bail!(
                    "timed out waiting for {markers:?}; last output: {}",
                    &self.buffer[start..]
                );
            }
            let mut chunk = [0u8; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => bail!("serial console closed while waiting for {markers:?}"),
                Ok(read) => self
                    .buffer
                    .push_str(&String::from_utf8_lossy(&chunk[..read])),
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(error) => return Err(error).context("failed to read serial console"),
            }
        }
    }

    pub(crate) fn run_command(&mut self, command: &str, timeout: Duration) -> Result<String> {
        let command_sequence = self.command_sequence;
        let marker = format!("QOL-RC-{}-{command_sequence}-", self.marker_nonce);
        self.command_sequence = self.command_sequence.wrapping_add(1);
        let emit_marker = format!(
            "qol_rc=$?; qol_i=0; while [ \"$qol_i\" -lt {EXIT_MARKER_COPIES} ]; do printf '\\n%s%s%s\\n' QOL- RC-{}-{}- \"$qol_rc\"; qol_i=$((qol_i+1)); done",
            self.marker_nonce,
            command_sequence
        );
        self.send_line(&format!("{command}; {emit_marker}"))?;
        let (output, code) = self.wait_for_exit_marker(&marker, timeout)?;
        if code != 0 {
            bail!("`{command}` exited {code}; output: {output}");
        }
        Ok(output)
    }

    fn wait_for_exit_marker(&mut self, marker: &str, timeout: Duration) -> Result<(String, u8)> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some((end, code)) = complete_exit_marker(&self.buffer, marker) {
                let consumed: String = self.buffer.drain(..end).collect();
                return Ok((consumed, code));
            }
            if Instant::now() >= deadline {
                let mut start = self.buffer.len().saturating_sub(600);
                while !self.buffer.is_char_boundary(start) {
                    start -= 1;
                }
                bail!(
                    "timed out waiting for exit marker {marker}; last output: {}",
                    &self.buffer[start..]
                );
            }
            let mut chunk = [0u8; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => bail!("serial console closed while waiting for exit marker {marker}"),
                Ok(read) => self
                    .buffer
                    .push_str(&String::from_utf8_lossy(&chunk[..read])),
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(error) => return Err(error).context("failed to read serial console"),
            }
        }
    }
}

fn new_marker_nonce() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SERIAL_NONCE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{timestamp:x}-{sequence:x}", std::process::id())
}

fn complete_exit_marker(buffer: &str, marker: &str) -> Option<(usize, u8)> {
    for (position, _) in buffer.match_indices(marker) {
        if !is_line_start(buffer, position) {
            continue;
        }
        let code_start = position + marker.len();
        let code_end = buffer[code_start..]
            .find(|character: char| !character.is_ascii_digit())
            .map(|offset| code_start + offset);
        let Some(code_end) = code_end else {
            continue;
        };
        if code_end == code_start {
            continue;
        }
        if !matches!(buffer.as_bytes()[code_end], b'\r' | b'\n') {
            continue;
        }
        let Ok(code) = buffer[code_start..code_end].parse::<u8>() else {
            continue;
        };
        return Some((code_end + 1, code));
    }
    None
}

fn is_line_start(buffer: &str, position: usize) -> bool {
    position == 0 || matches!(buffer.as_bytes()[position - 1], b'\r' | b'\n')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io;

    struct MockStream {
        reads: VecDeque<Vec<u8>>,
        written: Vec<u8>,
    }

    impl MockStream {
        fn new(reads: &[&str]) -> Self {
            Self {
                reads: reads.iter().map(|s| s.as_bytes().to_vec()).collect(),
                written: Vec::new(),
            }
        }
    }

    impl Read for MockStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(mut chunk) = self.reads.pop_front() else {
                return Ok(0);
            };
            let n = chunk.len().min(buf.len());
            buf[..n].copy_from_slice(&chunk[..n]);
            if n < chunk.len() {
                self.reads.push_front(chunk.split_off(n));
            }
            Ok(n)
        }
    }

    impl Write for MockStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn client(reads: &[&str]) -> SerialClient<MockStream> {
        SerialClient::from_transport(MockStream::new(reads))
    }

    const TIMEOUT: Duration = Duration::from_secs(1);
    const FIRST_MARKER: &str = "QOL-RC-test-0-";

    #[test]
    fn wait_for_finds_marker_split_across_reads() {
        let mut client = client(&["hello wor", "ld!\nmore"]);
        let consumed = client.wait_for("world", TIMEOUT).unwrap();
        assert_eq!(consumed, "hello world");
    }

    #[test]
    fn wait_for_any_reports_which_marker_matched() {
        let mut client = client(&["boot noise\nlogin: "]);
        let (index, consumed) = client.wait_for_any(&[":~#", "login:"], TIMEOUT).unwrap();
        assert_eq!(index, 1, "consumed: {consumed}");
        assert!(consumed.ends_with("login:"), "consumed: {consumed}");
    }

    #[test]
    fn run_command_ignores_echo_and_checks_exit_code() {
        let mut client = client(&[
            "ls /tmp; marker command echo\r\n",
            "file-a\r\nQOL-RC-test-0-0\r\n",
        ]);
        let output = client.run_command("ls /tmp", TIMEOUT).unwrap();
        assert!(output.contains("file-a"), "output: {output}");
        let written = String::from_utf8_lossy(&client.transport().written);
        assert!(
            written.starts_with("ls /tmp; qol_rc=$?;"),
            "written: {written}"
        );
        assert!(!written.contains(FIRST_MARKER), "written: {written}");
    }

    #[test]
    fn run_command_fails_on_nonzero_exit() {
        let mut client = client(&["\r\nQOL-RC-test-0-1\r\n"]);
        let error = client.run_command("false", TIMEOUT).unwrap_err();
        assert!(error.to_string().contains("exited 1"), "error: {error}");
    }

    #[test]
    fn run_command_ignores_kernel_log_appended_before_marker_newline() {
        let mut client = client(&[
            "device wait output\r\nQOL-RC-test-0-0[    9.131204] sd 6:0:0:0: [sda] Attached SCSI disk\r\n",
            "\r\nQOL-R",
            "C-test-0-0\r",
            "\n",
        ]);
        let output = client.run_command("wait-for-stick", TIMEOUT).unwrap();
        assert!(output.contains("Attached SCSI disk"), "output: {output}");
    }

    #[test]
    fn run_command_ignores_kernel_log_inserted_inside_marker() {
        let mut client = client(&[
            "\r\nQOL-RC-test-0-[    8.096807] scsi host6: usb-storage 2-1:1.0\r\n0\r\n",
            "\r\nQOL-RC-test-0-0\r\n",
        ]);
        client.run_command("wait-for-stick", TIMEOUT).unwrap();
    }

    #[test]
    fn run_command_rejects_partial_and_malformed_zero_before_nonzero_exit() {
        let mut client = client(&[
            "\r\nQOL-RC-test-0-\r\n",
            "xQOL-RC-test-0-0\r\n",
            "\r\nQOL-RC-test-0-0oops\r\n",
            "\r\nQOL-RC-test-0-256\r\n",
            "\r\nQOL-RC-test-0-999999999999999999999\r\n",
            "\r\nQOL-RC-test-0-7\r\n",
        ]);
        let error = client.run_command("failing-command", TIMEOUT).unwrap_err();
        assert!(error.to_string().contains("exited 7"), "error: {error}");
    }

    #[test]
    fn run_command_rejects_partial_marker_when_console_closes() {
        let mut client = client(&["\r\nQOL-RC-test-0-"]);
        let error = client.run_command("command", TIMEOUT).unwrap_err();
        assert!(
            error.to_string().contains("serial console closed"),
            "error: {error}"
        );
    }

    #[test]
    fn run_command_does_not_reuse_redundant_marker_from_previous_command() {
        let mut client = client(&[
            "\r\nQOL-RC-test-0-0\r\nQOL-RC-test-0-0\r\n",
            "\r\nQOL-RC-test-1-4\r\n",
        ]);
        client.run_command("first", TIMEOUT).unwrap();
        let error = client.run_command("second", TIMEOUT).unwrap_err();
        assert!(error.to_string().contains("exited 4"), "error: {error}");
    }
}
