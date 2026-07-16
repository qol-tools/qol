use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

const QMP_ABSOLUTE_MAX: u64 = 0x7fff;
const POINTER_CLICK_HOLD: Duration = Duration::from_millis(50);

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

pub(crate) struct QmpClient<S = TcpStream> {
    reader: BufReader<S>,
    pending_events: Vec<String>,
    pub(crate) qemu_version: String,
}

pub(crate) fn connect(port: u16, timeout: Duration) -> Result<QmpClient> {
    let deadline = Instant::now() + timeout;
    let address = format!("127.0.0.1:{port}");
    loop {
        match TcpStream::connect(&address) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .context("failed to set qmp read timeout")?;
                return QmpClient::handshake(stream);
            }
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

pub(crate) fn connect_verified(port: u16, timeout: Duration, run_id: &str) -> Result<QmpClient> {
    let mut client = connect(port, timeout)?;
    client.verify_run_identity(run_id)?;
    Ok(client)
}

pub(crate) fn pixel_to_qmp_absolute(pixel: u32, extent: u32) -> Result<u16> {
    if extent == 0 {
        bail!("pointer extent must be positive");
    }
    if pixel >= extent {
        bail!("pointer pixel {pixel} is outside extent {extent}");
    }
    if extent == 1 {
        return Ok(0);
    }
    let scaled = u64::from(pixel) * QMP_ABSOLUTE_MAX / u64::from(extent - 1);
    u16::try_from(scaled).context("absolute pointer coordinate exceeds the QMP range")
}

impl<S: Read + Write> QmpClient<S> {
    fn handshake(stream: S) -> Result<Self> {
        let mut client = Self {
            reader: BufReader::new(stream),
            pending_events: Vec::new(),
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
        writeln!(self.reader.get_mut(), "{request}")
            .with_context(|| format!("failed to send qmp command {command}"))?;
        loop {
            let line = self.read_line()?;
            match classify_line(&line)? {
                QmpLine::Return(value) => return Ok(value),
                QmpLine::Event(event) => self.pending_events.push(event),
                QmpLine::Greeting { .. } => bail!("unexpected qmp greeting mid-session"),
                QmpLine::Error(error) => bail!("qmp {command} failed: {error}"),
            }
        }
    }

    pub(crate) fn fire(&mut self, command: &str) -> Result<()> {
        let request = serde_json::json!({ "execute": command });
        writeln!(self.reader.get_mut(), "{request}")
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
        if let Some(index) = self.pending_events.iter().position(|event| event == name) {
            self.pending_events.remove(index);
            return Ok(());
        }
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let line = self.read_line()?;
            match classify_line(&line)? {
                QmpLine::Event(event) if event == name => return Ok(()),
                QmpLine::Event(event) => self.pending_events.push(event),
                QmpLine::Return(_) => continue,
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

    pub(crate) fn move_pointer_absolute(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let x = pixel_to_qmp_absolute(x, width)?;
        let y = pixel_to_qmp_absolute(y, height)?;
        self.send_input_events(vec![
            serde_json::json!({"type": "abs", "data": {"axis": "x", "value": x}}),
            serde_json::json!({"type": "abs", "data": {"axis": "y", "value": y}}),
        ])
    }

    pub(crate) fn set_left_button(&mut self, down: bool) -> Result<()> {
        self.send_input_events(vec![left_button_event(down)])
    }

    pub(crate) fn click_left(&mut self) -> Result<()> {
        self.set_left_button(true)?;
        std::thread::sleep(POINTER_CLICK_HOLD);
        self.set_left_button(false)
    }

    fn send_input_events(&mut self, events: Vec<Value>) -> Result<()> {
        self.execute(
            "input-send-event",
            Some(serde_json::json!({"events": events})),
        )?;
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

    pub(crate) fn query_machine_name(&mut self) -> Result<String> {
        let value = self.execute("query-name", None)?;
        value
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("query-name returned no machine name: {value}"))
    }

    pub(crate) fn verify_run_identity(&mut self, run_id: &str) -> Result<()> {
        let expected = format!("qol-emu-{run_id}");
        let actual = self.query_machine_name()?;
        if actual != expected {
            bail!("qmp machine identity mismatch: expected `{expected}`, got `{actual}`");
        }
        Ok(())
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

fn left_button_event(down: bool) -> Value {
    serde_json::json!({"type": "btn", "data": {"button": "left", "down": down}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    const GREETING: &str =
        r#"{"QMP":{"version":{"qemu":{"major":9,"minor":2,"micro":0}},"capabilities":[]}}"#;

    struct MockStream {
        reads: Vec<u8>,
        pos: usize,
        written: Vec<u8>,
    }

    impl Read for MockStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = (self.reads.len() - self.pos).min(buf.len());
            buf[..n].copy_from_slice(&self.reads[self.pos..self.pos + n]);
            self.pos += n;
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

    fn client(replies: &[&str]) -> QmpClient<MockStream> {
        let mut script = String::new();
        script.push_str(GREETING);
        script.push('\n');
        script.push_str(r#"{"return":{}}"#);
        script.push('\n');
        for reply in replies {
            script.push_str(reply);
            script.push('\n');
        }
        QmpClient::handshake(MockStream {
            reads: script.into_bytes(),
            pos: 0,
            written: Vec::new(),
        })
        .unwrap()
    }

    fn requests(client: &QmpClient<MockStream>) -> Vec<Value> {
        String::from_utf8(client.reader.get_ref().written.clone())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn send_keys_builds_qcode_chord() {
        let mut client = client(&[r#"{"return":{}}"#]);
        client
            .send_keys(&["ctrl".to_string(), "c".to_string()])
            .unwrap();
        let request = &requests(&client)[1];
        assert_eq!(request["execute"], "send-key");
        let chord: Vec<(&str, &str)> = request["arguments"]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| (key["type"].as_str().unwrap(), key["data"].as_str().unwrap()))
            .collect();
        assert_eq!(chord, [("qcode", "ctrl"), ("qcode", "c")]);
    }

    #[test]
    fn pixel_coordinates_cover_the_full_qmp_absolute_range() {
        let cases = [
            (0, 1920, 0),
            (1919, 1920, 0x7fff),
            (1, 3, 0x3fff),
            (0, 1, 0),
            (u32::MAX - 1, u32::MAX, 0x7fff),
        ];
        for (pixel, extent, expected) in cases {
            assert_eq!(
                pixel_to_qmp_absolute(pixel, extent).unwrap(),
                expected,
                "pixel: {pixel}, extent: {extent}"
            );
        }
    }

    #[test]
    fn pixel_coordinates_reject_empty_extents_and_out_of_bounds_pixels() {
        let cases = [(0, 0), (1, 1), (1920, 1920), (u32::MAX, u32::MAX)];
        for (pixel, extent) in cases {
            assert!(
                pixel_to_qmp_absolute(pixel, extent).is_err(),
                "pixel: {pixel}, extent: {extent}"
            );
        }
    }

    #[test]
    fn absolute_pointer_move_sends_scaled_axes_in_one_qmp_request() {
        let mut client = client(&[r#"{"return":{}}"#]);
        client.move_pointer_absolute(0, 1079, 1920, 1080).unwrap();

        let request = &requests(&client)[1];
        assert_eq!(request["execute"], "input-send-event");
        assert_eq!(
            request["arguments"]["events"],
            serde_json::json!([
                {"type": "abs", "data": {"axis": "x", "value": 0}},
                {"type": "abs", "data": {"axis": "y", "value": 0x7fff}}
            ])
        );
    }

    #[test]
    fn invalid_absolute_pointer_move_sends_no_input_request() {
        let mut client = client(&[]);
        let error = client
            .move_pointer_absolute(1920, 0, 1920, 1080)
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "pointer pixel 1920 is outside extent 1920"
        );
        assert_eq!(requests(&client).len(), 1);
    }

    #[test]
    fn left_button_state_sends_down_and_up_events() {
        let mut client = client(&[r#"{"return":{}}"#, r#"{"return":{}}"#]);
        client.set_left_button(true).unwrap();
        client.set_left_button(false).unwrap();

        let requests = requests(&client);
        assert_eq!(requests[1]["execute"], "input-send-event");
        assert_eq!(requests[2]["execute"], "input-send-event");
        assert_eq!(
            requests[1]["arguments"]["events"],
            serde_json::json!([
                {"type": "btn", "data": {"button": "left", "down": true}}
            ])
        );
        assert_eq!(
            requests[2]["arguments"]["events"],
            serde_json::json!([
                {"type": "btn", "data": {"button": "left", "down": false}}
            ])
        );
    }

    #[test]
    fn click_left_sends_distinct_press_and_release_events() {
        let mut client = client(&[r#"{"return":{}}"#, r#"{"return":{}}"#]);
        client.click_left().unwrap();

        let requests = requests(&client);
        assert_eq!(requests[1]["execute"], "input-send-event");
        assert_eq!(
            requests[1]["arguments"]["events"],
            serde_json::json!([
                {"type": "btn", "data": {"button": "left", "down": true}}
            ])
        );
        assert_eq!(requests[2]["execute"], "input-send-event");
        assert_eq!(
            requests[2]["arguments"]["events"],
            serde_json::json!([
                {"type": "btn", "data": {"button": "left", "down": false}}
            ])
        );
    }

    #[test]
    fn screendump_sends_filename() {
        let mut client = client(&[r#"{"return":{}}"#]);
        client.screendump(Path::new("/a/b/shot.ppm")).unwrap();
        let request = &requests(&client)[1];
        assert_eq!(request["execute"], "screendump");
        assert_eq!(request["arguments"]["filename"], "/a/b/shot.ppm");
    }

    #[test]
    fn attach_usb_stick_adds_blockdev_then_device() {
        let mut client = client(&[r#"{"return":{}}"#, r#"{"return":{}}"#]);
        client
            .attach_usb_stick(Path::new("/a/b/usb-stick.raw"))
            .unwrap();
        let reqs = requests(&client);
        assert_eq!(reqs[1]["execute"], "blockdev-add");
        assert_eq!(reqs[1]["arguments"]["node-name"], "qolusb");
        assert_eq!(
            reqs[1]["arguments"]["file"]["filename"],
            "/a/b/usb-stick.raw"
        );
        assert_eq!(reqs[2]["execute"], "device_add");
        assert_eq!(reqs[2]["arguments"]["driver"], "usb-storage");
        assert_eq!(reqs[2]["arguments"]["bus"], "xhci.0");
    }

    #[test]
    fn detach_usb_stick_deletes_device_waits_then_drops_blockdev() {
        let mut client = client(&[
            "{\"event\":\"DEVICE_DELETED\",\"data\":{\"device\":\"qolusbdev\"},\"timestamp\":{\"seconds\":0,\"microseconds\":0}}\n{\"return\":{}}",
            r#"{"return":{}}"#,
        ]);
        client.detach_usb_stick().unwrap();
        let reqs = requests(&client);
        assert_eq!(reqs[1]["execute"], "device_del");
        assert_eq!(reqs[2]["execute"], "blockdev-del");
    }

    #[test]
    fn wait_event_skips_unrelated_lines_until_match() {
        let mut client = client(&[
            "{\"return\":{}}\n{\"event\":\"NIC_RX_FILTER_CHANGED\",\"timestamp\":{\"seconds\":0,\"microseconds\":0}}\n{\"event\":\"DEVICE_DELETED\",\"data\":{\"device\":\"qolusbdev\"},\"timestamp\":{\"seconds\":0,\"microseconds\":0}}",
        ]);
        client.execute("query-status", None).unwrap();
        client
            .wait_event("DEVICE_DELETED", Duration::from_secs(2))
            .unwrap();
        assert_eq!(requests(&client)[1]["execute"], "query-status");
    }

    #[test]
    fn disk_snapshot_targets_qoldisk() {
        let mut client = client(&[r#"{"return":{}}"#]);
        client
            .disk_snapshot(Path::new("/a/b/overlay-snap-1.qcow2"))
            .unwrap();
        let request = &requests(&client)[1];
        assert_eq!(request["execute"], "blockdev-snapshot-sync");
        assert_eq!(request["arguments"]["device"], "qoldisk");
        assert_eq!(
            request["arguments"]["snapshot-file"],
            "/a/b/overlay-snap-1.qcow2"
        );
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
        let mut client = client(&[
            r#"{"event":"NIC_RX_FILTER_CHANGED","timestamp":{"seconds":0,"microseconds":0}}"#,
            r#"{"return":{"status":"running","running":true}}"#,
        ]);
        assert_eq!(client.qemu_version, "9.2.0");
        assert_eq!(client.query_status().unwrap(), "running");
        let reqs = requests(&client);
        assert_eq!(reqs[0]["execute"], "qmp_capabilities");
        assert_eq!(reqs[1]["execute"], "query-status");
    }

    #[test]
    fn verifies_exact_run_identity() {
        let mut client = client(&[r#"{"return":{"name":"qol-emu-mint-a"}}"#]);
        client.verify_run_identity("mint-a").unwrap();
        let request = &requests(&client)[1];
        assert_eq!(request["execute"], "query-name");
    }

    #[test]
    fn rejects_mismatched_run_identity_before_other_commands() {
        let mut client = client(&[r#"{"return":{"name":"qol-emu-another-run"}}"#]);
        let error = client.verify_run_identity("mint-a").unwrap_err();
        assert_eq!(
            error.to_string(),
            "qmp machine identity mismatch: expected `qol-emu-mint-a`, got `qol-emu-another-run`"
        );
        let reqs = requests(&client);
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[1]["execute"], "query-name");
    }

    #[test]
    fn rejects_missing_machine_name() {
        for response in [r#"{"return":{}}"#, r#"{"return":{"name":""}}"#] {
            let mut client = client(&[response]);
            let error = client.verify_run_identity("mint-a").unwrap_err();
            assert!(
                error
                    .to_string()
                    .starts_with("query-name returned no machine name:"),
                "response: {response}, error: {error:#}"
            );
        }
    }

    #[test]
    fn execute_sends_arguments_payload() {
        let mut client = client(&[r#"{"return":{}}"#]);
        client
            .execute(
                "screendump",
                Some(serde_json::json!({"filename": "/a/b/shot.ppm"})),
            )
            .unwrap();
        let request = &requests(&client)[1];
        assert_eq!(request["execute"], "screendump");
        assert_eq!(request["arguments"]["filename"], "/a/b/shot.ppm");
    }
}
