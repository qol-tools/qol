use anyhow::{bail, Context, Result};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub(crate) struct SerialClient<S = TcpStream> {
    stream: S,
    buffer: String,
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
        self.send_line(&format!("{command}; echo QOL-\"RC\"-$?"))?;
        let output = self.wait_for("QOL-RC-", timeout)?;
        let code = self.wait_for("\n", Duration::from_secs(5))?;
        let code = code.trim();
        if code != "0" {
            bail!("`{command}` exited {code}; output: {output}");
        }
        Ok(output)
    }
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
        let mut client = client(&["ls /tmp; echo QOL-\"RC\"-$?\n", "file-a\nQOL-RC-0\n"]);
        let output = client.run_command("ls /tmp", TIMEOUT).unwrap();
        assert!(output.contains("file-a"), "output: {output}");
        assert_eq!(client.transport().written, b"ls /tmp; echo QOL-\"RC\"-$?\n");
    }

    #[test]
    fn run_command_fails_on_nonzero_exit() {
        let mut client = client(&["false; echo QOL-\"RC\"-$?\n", "QOL-RC-1\n"]);
        let error = client.run_command("false", TIMEOUT).unwrap_err();
        assert!(error.to_string().contains("exited 1"), "error: {error}");
    }
}
