use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) enum QmpLine {
    Greeting { qemu_version: String },
    Return(Value),
    Event(String),
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
    if let Some(event) = value.get("event") {
        return Ok(QmpLine::Event(
            event.as_str().unwrap_or_default().to_string(),
        ));
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
            QmpLine::Return(_) | QmpLine::Event(_) | QmpLine::Error(_) => {
                bail!("expected qmp greeting, got: {line}")
            }
        }
        client.execute("qmp_capabilities", None)?;
        Ok(client)
    }

    pub(crate) fn execute(&mut self, command: &str, arguments: Option<Value>) -> Result<Value> {
        let mut request = serde_json::json!({ "execute": command });
        if let Some(arguments) = arguments {
            request["arguments"] = arguments;
        }
        writeln!(self.stream, "{request}")
            .with_context(|| format!("failed to send qmp command {command}"))?;
        loop {
            let line = self.read_line()?;
            match classify_line(&line)? {
                QmpLine::Return(value) => return Ok(value),
                QmpLine::Event(_) => continue,
                QmpLine::Greeting { .. } => bail!("unexpected qmp greeting mid-session"),
                QmpLine::Error(error) => bail!("qmp {command} failed: {error}"),
            }
        }
    }

    pub(crate) fn fire(&mut self, command: &str) -> Result<()> {
        let request = serde_json::json!({ "execute": command });
        writeln!(self.stream, "{request}")
            .with_context(|| format!("failed to send qmp command {command}"))
    }

    pub(crate) fn screendump(&mut self, path: &Path) -> Result<()> {
        self.execute(
            "screendump",
            Some(serde_json::json!({"filename": path.display().to_string()})),
        )?;
        Ok(())
    }

    pub(crate) fn wait_event(&mut self, name: &str, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let line = self.read_line()?;
            match classify_line(&line)? {
                QmpLine::Event(event) if event == name => return Ok(()),
                QmpLine::Event(_) | QmpLine::Return(_) => continue,
                QmpLine::Greeting { .. } => bail!("unexpected qmp greeting mid-session"),
                QmpLine::Error(error) => {
                    bail!("qmp error while waiting for event {name}: {error}")
                }
            }
        }
        bail!("timed out waiting for qmp event {name}")
    }

    pub(crate) fn attach_usb_stick(&mut self, image: &Path) -> Result<()> {
        self.execute(
            "blockdev-add",
            Some(serde_json::json!({
                "driver": "raw",
                "node-name": "qolusb",
                "file": {"driver": "file", "filename": image.display().to_string()},
            })),
        )?;
        self.execute(
            "device_add",
            Some(serde_json::json!({
                "driver": "usb-storage",
                "id": "qolusbdev",
                "bus": "xhci.0",
                "drive": "qolusb",
            })),
        )?;
        Ok(())
    }

    pub(crate) fn detach_usb_stick(&mut self) -> Result<()> {
        self.execute("device_del", Some(serde_json::json!({"id": "qolusbdev"})))?;
        self.wait_event("DEVICE_DELETED", Duration::from_secs(5))?;
        self.execute(
            "blockdev-del",
            Some(serde_json::json!({"node-name": "qolusb"})),
        )?;
        Ok(())
    }

    pub(crate) fn disk_snapshot(&mut self, snapshot_file: &Path) -> Result<()> {
        self.execute(
            "blockdev-snapshot-sync",
            Some(serde_json::json!({
                "device": "qoldisk",
                "snapshot-file": snapshot_file.display().to_string(),
                "format": "qcow2",
            })),
        )?;
        Ok(())
    }

    pub(crate) fn send_keys(&mut self, keys: &[String]) -> Result<()> {
        let chord: Vec<Value> = keys
            .iter()
            .map(|key| serde_json::json!({"type": "qcode", "data": key}))
            .collect();
        self.execute("send-key", Some(serde_json::json!({"keys": chord})))?;
        Ok(())
    }

    pub(crate) fn query_status(&mut self) -> Result<String> {
        let value = self.execute("query-status", None)?;
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

    fn fake_server(
        replies: Vec<&'static str>,
        assert_lines: fn(usize, &str),
    ) -> (std::thread::JoinHandle<()>, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
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
            writeln!(stream, r#"{{"return":{{}}}}"#).unwrap();
            for (index, reply) in replies.into_iter().enumerate() {
                line.clear();
                reader.read_line(&mut line).unwrap();
                assert_lines(index, &line);
                writeln!(stream, "{reply}").unwrap();
            }
        });
        (handle, port)
    }

    #[test]
    fn send_keys_builds_qcode_chord() {
        let (server, port) = fake_server(vec![r#"{"return":{}}"#], |_, line| {
            assert!(line.contains(r#""execute":"send-key""#), "line: {line}");
            assert!(
                line.contains(r#"{"data":"ctrl","type":"qcode"}"#),
                "line: {line}"
            );
            assert!(
                line.contains(r#"{"data":"c","type":"qcode"}"#),
                "line: {line}"
            );
        });
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        client
            .send_keys(&["ctrl".to_string(), "c".to_string()])
            .unwrap();
        server.join().unwrap();
    }

    #[test]
    fn screendump_sends_filename() {
        let (server, port) = fake_server(vec![r#"{"return":{}}"#], |_, line| {
            assert!(line.contains(r#""execute":"screendump""#), "line: {line}");
            assert!(line.contains("shot.ppm"), "line: {line}");
        });
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        client.screendump(Path::new("/a/b/shot.ppm")).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn attach_usb_stick_adds_blockdev_then_device() {
        let (server, port) = fake_server(
            vec![r#"{"return":{}}"#, r#"{"return":{}}"#],
            |index, line| match index {
                0 => {
                    assert!(line.contains(r#""execute":"blockdev-add""#), "line: {line}");
                    assert!(line.contains(r#""node-name":"qolusb""#), "line: {line}");
                    assert!(line.contains("usb-stick.raw"), "line: {line}");
                }
                1 => {
                    assert!(line.contains(r#""execute":"device_add""#), "line: {line}");
                    assert!(line.contains(r#""driver":"usb-storage""#), "line: {line}");
                    assert!(line.contains(r#""bus":"xhci.0""#), "line: {line}");
                }
                other => panic!("unexpected command index {other}: {line}"),
            },
        );
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        client
            .attach_usb_stick(Path::new("/a/b/usb-stick.raw"))
            .unwrap();
        server.join().unwrap();
    }

    #[test]
    fn detach_usb_stick_deletes_device_waits_then_drops_blockdev() {
        let (server, port) = fake_server(
            vec![
                "{\"return\":{}}\n{\"event\":\"DEVICE_DELETED\",\"data\":{\"device\":\"qolusbdev\"},\"timestamp\":{\"seconds\":0,\"microseconds\":0}}",
                r#"{"return":{}}"#,
            ],
            |index, line| match index {
                0 => assert!(line.contains(r#""execute":"device_del""#), "line: {line}"),
                1 => assert!(line.contains(r#""execute":"blockdev-del""#), "line: {line}"),
                other => panic!("unexpected command index {other}: {line}"),
            },
        );
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        client.detach_usb_stick().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn wait_event_skips_unrelated_lines_until_match() {
        let (server, port) = fake_server(
            vec![
                "{\"return\":{}}\n{\"event\":\"NIC_RX_FILTER_CHANGED\",\"timestamp\":{\"seconds\":0,\"microseconds\":0}}\n{\"event\":\"DEVICE_DELETED\",\"data\":{\"device\":\"qolusbdev\"},\"timestamp\":{\"seconds\":0,\"microseconds\":0}}",
            ],
            |_, line| assert!(line.contains(r#""execute":"query-status""#), "line: {line}"),
        );
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        client.execute("query-status", None).unwrap();
        client
            .wait_event("DEVICE_DELETED", Duration::from_secs(2))
            .unwrap();
        server.join().unwrap();
    }

    #[test]
    fn disk_snapshot_targets_qoldisk() {
        let (server, port) = fake_server(vec![r#"{"return":{}}"#], |_, line| {
            assert!(
                line.contains(r#""execute":"blockdev-snapshot-sync""#),
                "line: {line}"
            );
            assert!(line.contains(r#""device":"qoldisk""#), "line: {line}");
            assert!(line.contains("overlay-snap"), "line: {line}");
        });
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        client
            .disk_snapshot(Path::new("/a/b/overlay-snap-1.qcow2"))
            .unwrap();
        server.join().unwrap();
    }

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
                "event POWERDOWN",
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
                QmpLine::Event(name) => format!("event {name}"),
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

    #[test]
    fn execute_sends_arguments_payload() {
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
            writeln!(stream, r#"{{"return":{{}}}}"#).unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.contains(r#""execute":"screendump""#), "line: {line}");
            assert!(
                line.contains(r#""filename":"/a/b/shot.ppm""#),
                "line: {line}"
            );
            writeln!(stream, r#"{{"return":{{}}}}"#).unwrap();
        });
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        client
            .execute(
                "screendump",
                Some(serde_json::json!({"filename": "/a/b/shot.ppm"})),
            )
            .unwrap();
        server.join().unwrap();
    }
}
