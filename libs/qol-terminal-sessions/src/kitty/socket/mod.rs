mod platform;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{Map, Value};

use super::CommandOutput;

const FRAME_PREFIX: &[u8] = b"\x1bP@kitty-cmd";
const FRAME_SUFFIX: &[u8] = b"\x1b\\";
const OLDEST_SUPPORTED_PROTOCOL_VERSION: [u32; 3] = [0, 14, 0];
const COMMANDS_THAT_LEAVE_TERMINALS_UNCHANGED: [&str; 2] = ["ls", "get-text"];

pub(super) fn try_run(args: &[String], stdin: Option<&str>) -> Option<CommandOutput> {
    if stdin.is_some() {
        return None;
    }
    let path = socket_path()?;
    let request = request(args)?;
    match send(path, &request) {
        Ok(output) => Some(output),
        Err(error) => {
            qol_runtime::probe!(
                "TERMINAL_SESSIONS",
                "backend=kitty transport=socket outcome=fallback_to_kitten error={error}"
            );
            None
        }
    }
}

fn socket_path() -> Option<&'static Path> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| connectable_socket_from_listen_on(&std::env::var("KITTY_LISTEN_ON").ok()?))
        .as_deref()
}

fn connectable_socket_from_listen_on(value: &str) -> Option<PathBuf> {
    let path = value.strip_prefix("unix:")?;
    if path.is_empty() || path.starts_with('@') {
        return None;
    }
    Some(PathBuf::from(path))
}

fn request(args: &[String]) -> Option<Vec<u8>> {
    let (marker, rest) = args.split_first()?;
    if marker != "@" {
        return None;
    }
    let (name, flags) = rest.split_first()?;
    if !COMMANDS_THAT_LEAVE_TERMINALS_UNCHANGED.contains(&name.as_str()) {
        return None;
    }
    let payload = payload_from_flags(flags)?;

    let mut command = Map::new();
    command.insert("cmd".to_owned(), Value::String(name.clone()));
    command.insert(
        "version".to_owned(),
        serde_json::to_value(OLDEST_SUPPORTED_PROTOCOL_VERSION).ok()?,
    );
    command.insert("no_response".to_owned(), Value::Bool(false));
    if !payload.is_empty() {
        command.insert("payload".to_owned(), Value::Object(payload));
    }

    let body = serde_json::to_vec(&Value::Object(command)).ok()?;
    let mut request = Vec::with_capacity(FRAME_PREFIX.len() + body.len() + FRAME_SUFFIX.len());
    request.extend_from_slice(FRAME_PREFIX);
    request.extend_from_slice(&body);
    request.extend_from_slice(FRAME_SUFFIX);
    Some(request)
}

fn payload_from_flags(flags: &[String]) -> Option<Map<String, Value>> {
    if !flags.len().is_multiple_of(2) {
        return None;
    }
    let mut payload = Map::new();
    for pair in flags.chunks_exact(2) {
        let key = pair[0].strip_prefix("--")?;
        if key.contains('=') {
            return None;
        }
        payload.insert(key.replace('-', "_"), Value::String(pair[1].clone()));
    }
    Some(payload)
}

fn send(path: &Path, request: &[u8]) -> std::io::Result<CommandOutput> {
    decode(&platform::exchange(path, request, FRAME_SUFFIX)?)
}

fn decode(response: &[u8]) -> std::io::Result<CommandOutput> {
    let body = response
        .strip_prefix(FRAME_PREFIX)
        .and_then(|rest| rest.strip_suffix(FRAME_SUFFIX))
        .ok_or_else(|| std::io::Error::other("Kitty socket reply is not a kitty-cmd frame"))?;
    let reply: Value = serde_json::from_slice(body).map_err(std::io::Error::other)?;

    if reply.get("ok").and_then(Value::as_bool) != Some(true) {
        let message = reply
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Kitty rejected the command");
        return Ok(CommandOutput {
            success: false,
            code: Some(1),
            stdout: String::new(),
            stderr: message.to_owned(),
        });
    }
    Ok(CommandOutput {
        success: true,
        code: Some(0),
        stdout: reply
            .get("data")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        stderr: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::{connectable_socket_from_listen_on, decode, request, FRAME_PREFIX, FRAME_SUFFIX};

    fn strings<const N: usize>(values: [&str; N]) -> Vec<String> {
        values.into_iter().map(str::to_owned).collect()
    }

    fn decoded_request<const N: usize>(args: [&str; N]) -> Value {
        let bytes = request(&strings(args)).expect("args are socket eligible");
        let body = bytes
            .strip_prefix(FRAME_PREFIX)
            .and_then(|rest| rest.strip_suffix(FRAME_SUFFIX))
            .expect("request carries the kitty-cmd frame");
        serde_json::from_slice(body).expect("request body is JSON")
    }

    fn framed(body: Value) -> Vec<u8> {
        let mut bytes = FRAME_PREFIX.to_vec();
        bytes.extend_from_slice(&serde_json::to_vec(&body).unwrap());
        bytes.extend_from_slice(FRAME_SUFFIX);
        bytes
    }

    #[test]
    fn discovery_encodes_without_a_payload() {
        assert_eq!(
            decoded_request(["@", "ls"]),
            json!({"cmd": "ls", "version": [0, 14, 0], "no_response": false})
        );
    }

    #[test]
    fn screen_reads_encode_their_flags_as_payload_keys() {
        assert_eq!(
            decoded_request(["@", "get-text", "--match", "id:42", "--extent", "screen"]),
            json!({
                "cmd": "get-text",
                "version": [0, 14, 0],
                "no_response": false,
                "payload": {"match": "id:42", "extent": "screen"},
            })
        );
    }

    #[test]
    fn flag_names_become_snake_case_payload_keys() {
        let encoded = decoded_request(["@", "get-text", "--add-cursor", "yes"]);
        assert_eq!(encoded["payload"], json!({"add_cursor": "yes"}));
    }

    #[test]
    fn commands_that_change_a_terminal_are_declined() {
        for args in [
            strings(["@", "send-key", "--match", "id:42", "enter"]),
            strings(["@", "send-text", "--match", "id:42", "--stdin"]),
            strings(["@", "focus-window", "--match", "id:42"]),
        ] {
            assert!(
                request(&args).is_none(),
                "{args:?} must fall back to the kitten binary"
            );
        }
    }

    #[test]
    fn unparseable_flag_shapes_are_declined() {
        for args in [
            strings(["@", "get-text", "--match"]),
            strings(["@", "get-text", "--bracketed-paste=auto", "x"]),
            strings(["@", "get-text", "screen", "--extent"]),
            strings(["ls"]),
            strings(["@"]),
        ] {
            assert!(
                request(&args).is_none(),
                "{args:?} must fall back to the kitten binary"
            );
        }
    }

    #[test]
    fn a_successful_reply_becomes_stdout() {
        let output = decode(&framed(json!({"ok": true, "data": "[{\"id\":1}]"}))).unwrap();

        assert!(output.success);
        assert_eq!(output.code, Some(0));
        assert_eq!(output.stdout, "[{\"id\":1}]");
        assert!(output.stderr.is_empty());
    }

    #[test]
    fn a_reply_without_data_yields_empty_stdout() {
        let output = decode(&framed(json!({"ok": true}))).unwrap();

        assert!(output.success);
        assert!(output.stdout.is_empty());
    }

    #[test]
    fn a_rejected_command_reports_the_kitty_error() {
        let output = decode(&framed(
            json!({"ok": false, "error": "No matching windows for expression: id:9"}),
        ))
        .unwrap();

        assert!(!output.success);
        assert_eq!(output.code, Some(1));
        assert_eq!(output.stderr, "No matching windows for expression: id:9");
        assert!(output.stdout.is_empty());
    }

    #[test]
    fn a_truncated_or_unframed_reply_is_an_error() {
        for reply in [
            b"{\"ok\": true}".to_vec(),
            FRAME_PREFIX.to_vec(),
            Vec::new(),
            framed(json!({"ok": true}))[..12].to_vec(),
        ] {
            assert!(
                decode(&reply).is_err(),
                "an unframed reply must not be read as success"
            );
        }
    }

    #[test]
    fn a_framed_reply_that_is_not_json_is_an_error() {
        let mut bytes = FRAME_PREFIX.to_vec();
        bytes.extend_from_slice(b"not json");
        bytes.extend_from_slice(FRAME_SUFFIX);

        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn only_filesystem_sockets_are_accepted() {
        assert_eq!(
            connectable_socket_from_listen_on("unix:/tmp/kitty-5002.sock"),
            Some("/tmp/kitty-5002.sock".into())
        );
        for value in ["unix:@kitty", "tcp:localhost:4321", "unix:", "/tmp/x.sock"] {
            assert!(
                connectable_socket_from_listen_on(value).is_none(),
                "{value} is not reachable with UnixStream::connect"
            );
        }
    }
}
