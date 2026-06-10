use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) enum QmpLine {
    Greeting { qemu_version: String },
    Return(Value),
    Event,
    Error(String),
}

pub(crate) fn classify_line(line: &str) -> Result<QmpLine> {
    let value: Value =
        serde_json::from_str(line).with_context(|| format!("qmp line is not JSON: {line}"))?;
    if value.get("QMP").is_some() {
        return Ok(QmpLine::Greeting {
            qemu_version: greeting_version(&value),
        });
    }
    if value.get("event").is_some() {
        return Ok(QmpLine::Event);
    }
    if let Some(error) = value.get("error") {
        return Ok(QmpLine::Error(error.to_string()));
    }
    if let Some(result) = value.get("return") {
        return Ok(QmpLine::Return(result.clone()));
    }
    bail!("unrecognized qmp line: {line}")
}

fn greeting_version(value: &Value) -> String {
    let qemu = value
        .get("QMP")
        .and_then(|qmp| qmp.get("version"))
        .and_then(|version| version.get("qemu"));
    let part = |key: &str| {
        qemu.and_then(|qemu| qemu.get(key))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    format!("{}.{}.{}", part("major"), part("minor"), part("micro"))
}

pub(crate) struct QmpClient {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    pub(crate) qemu_version: String,
}

pub(crate) fn connect(port: u16, timeout: Duration) -> Result<QmpClient> {
    let deadline = Instant::now() + timeout;
    let address = format!("127.0.0.1:{port}");
    loop {
        match TcpStream::connect(&address) {
            Ok(stream) => return QmpClient::handshake(stream),
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(error)
                        .with_context(|| format!("qmp connect to {address} timed out"));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

impl QmpClient {
    fn handshake(stream: TcpStream) -> Result<Self> {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .context("failed to set qmp read timeout")?;
        let reader = BufReader::new(stream.try_clone().context("failed to clone qmp stream")?);
        let mut client = Self {
            stream,
            reader,
            qemu_version: String::new(),
        };
        let line = client.read_line()?;
        match classify_line(&line)? {
            QmpLine::Greeting { qemu_version } => client.qemu_version = qemu_version,
            QmpLine::Return(_) | QmpLine::Event | QmpLine::Error(_) => {
                bail!("expected qmp greeting, got: {line}")
            }
        }
        client.execute("qmp_capabilities")?;
        Ok(client)
    }

    fn execute(&mut self, command: &str) -> Result<Value> {
        let request = serde_json::json!({ "execute": command });
        writeln!(self.stream, "{request}")
            .with_context(|| format!("failed to send qmp command {command}"))?;
        loop {
            let line = self.read_line()?;
            match classify_line(&line)? {
                QmpLine::Return(value) => return Ok(value),
                QmpLine::Event => continue,
                QmpLine::Greeting { .. } => bail!("unexpected qmp greeting mid-session"),
                QmpLine::Error(error) => bail!("qmp {command} failed: {error}"),
            }
        }
    }

    pub(crate) fn query_status(&mut self) -> Result<String> {
        let value = self.execute("query-status")?;
        value
            .get("status")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| anyhow!("query-status returned no status: {value}"))
    }

    fn read_line(&mut self) -> Result<String> {
        let mut line = String::new();
        let bytes = self
            .reader
            .read_line(&mut line)
            .context("failed to read qmp line")?;
        if bytes == 0 {
            bail!("qmp connection closed");
        }
        Ok(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn classifies_qmp_lines() {
        let cases = [
            (
                r#"{"QMP":{"version":{"qemu":{"major":9,"minor":2,"micro":1}},"capabilities":[]}}"#,
                "greeting 9.2.1",
            ),
            (r#"{"return":{}}"#, "return"),
            (
                r#"{"event":"POWERDOWN","timestamp":{"seconds":0,"microseconds":0}}"#,
                "event",
            ),
            (
                r#"{"error":{"class":"GenericError","desc":"nope"}}"#,
                "error",
            ),
        ];
        for (line, expected) in cases {
            let label = match classify_line(line).unwrap() {
                QmpLine::Greeting { qemu_version } => format!("greeting {qemu_version}"),
                QmpLine::Return(_) => "return".to_string(),
                QmpLine::Event => "event".to_string(),
                QmpLine::Error(_) => "error".to_string(),
            };
            assert_eq!(label, expected, "line: {line}");
        }
    }

    #[test]
    fn rejects_garbage_lines() {
        for line in ["not json", r#"{"unrelated":1}"#] {
            assert!(classify_line(line).is_err(), "should reject: {line}");
        }
    }

    #[test]
    fn connects_negotiates_and_queries_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut stream = stream;
            writeln!(
                stream,
                r#"{{"QMP":{{"version":{{"qemu":{{"major":9,"minor":2,"micro":0}}}},"capabilities":[]}}}}"#
            )
            .unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.contains("qmp_capabilities"), "first command: {line}");
            writeln!(stream, r#"{{"return":{{}}}}"#).unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.contains("query-status"), "second command: {line}");
            writeln!(
                stream,
                r#"{{"event":"NIC_RX_FILTER_CHANGED","timestamp":{{"seconds":0,"microseconds":0}}}}"#
            )
            .unwrap();
            writeln!(
                stream,
                r#"{{"return":{{"status":"running","running":true}}}}"#
            )
            .unwrap();
        });
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        assert_eq!(client.qemu_version, "9.2.0");
        assert_eq!(client.query_status().unwrap(), "running");
        server.join().unwrap();
    }
}
