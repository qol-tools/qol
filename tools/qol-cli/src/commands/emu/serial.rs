use anyhow::{bail, Context, Result};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub(crate) struct SerialClient {
    stream: TcpStream,
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

impl SerialClient {
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
                let mut start = self.buffer.len().saturating_sub(200);
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
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    fn streaming_server(chunks: Vec<(&'static str, u64)>) -> (std::thread::JoinHandle<()>, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            for (chunk, delay_ms) in chunks {
                std::thread::sleep(Duration::from_millis(delay_ms));
                stream.write_all(chunk.as_bytes()).unwrap();
            }
        });
        (handle, port)
    }

    fn echoing_server(
        replies: Vec<&'static str>,
        assert_lines: fn(usize, &str),
    ) -> (std::thread::JoinHandle<()>, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut stream = stream;
            for (index, reply) in replies.into_iter().enumerate() {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_lines(index, &line);
                stream.write_all(line.as_bytes()).unwrap();
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });
        (handle, port)
    }

    #[test]
    fn wait_for_finds_marker_split_across_reads() {
        let (server, port) = streaming_server(vec![("hello wor", 0), ("ld!\nmore", 50)]);
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        let consumed = client.wait_for("world", Duration::from_secs(2)).unwrap();
        assert_eq!(consumed, "hello world");
        server.join().unwrap();
    }

    #[test]
    fn wait_for_any_reports_which_marker_matched() {
        let (server, port) = streaming_server(vec![("boot noise\nlogin: ", 0)]);
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        let (index, consumed) = client
            .wait_for_any(&[":~#", "login:"], Duration::from_secs(2))
            .unwrap();
        assert_eq!(index, 1, "consumed: {consumed}");
        assert!(consumed.ends_with("login:"), "consumed: {consumed}");
        server.join().unwrap();
    }

    #[test]
    fn run_command_ignores_echo_and_checks_exit_code() {
        let (server, port) = echoing_server(vec!["file-a\nQOL-RC-0\n"], |_, line| {
            assert_eq!(line, "ls /tmp; echo QOL-\"RC\"-$?\n");
        });
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        let output = client
            .run_command("ls /tmp", Duration::from_secs(2))
            .unwrap();
        assert!(output.contains("file-a"), "output: {output}");
        server.join().unwrap();
    }

    #[test]
    fn run_command_fails_on_nonzero_exit() {
        let (server, port) = echoing_server(vec!["QOL-RC-1\n"], |_, _| {});
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        let error = client
            .run_command("false", Duration::from_secs(2))
            .unwrap_err();
        assert!(error.to_string().contains("exited 1"), "error: {error}");
        server.join().unwrap();
    }
}
