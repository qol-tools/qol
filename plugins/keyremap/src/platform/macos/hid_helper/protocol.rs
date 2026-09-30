use std::io::{self, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const SOCKET_PATH: &str = "/var/run/com.qol-tools.keyremap.hid-helper.sock";

pub(crate) const PAGE_KEYBOARD: u16 = 0x07;
pub(crate) const PAGE_CONSUMER: u16 = 0x0C;
pub(crate) const PAGE_APPLE_VENDOR_KEYBOARD: u16 = 0xFF01;
pub(crate) const PAGE_APPLE_VENDOR_TOP_CASE: u16 = 0x00FF;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Session,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ToHelper {
    Hello {
        protocol: u32,
        role: Role,
    },
    Heartbeat,
    Emit {
        usage_page: u16,
        usage: u16,
        pressed: bool,
    },
    CapsLockLight {
        on: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ToDaemon {
    Welcome {
        protocol: u32,
    },
    Refused {
        protocol: u32,
        reason: String,
    },
    Key {
        usage_page: u16,
        usage: u16,
        pressed: bool,
        apple: bool,
    },
    Seized {
        active: bool,
    },
    Status(HelperStatus),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HelperStatus {
    pub(crate) protocol: u32,
    pub(crate) virtual_keyboard_ready: bool,
    pub(crate) input_monitoring: bool,
    pub(crate) seized: Vec<String>,
    pub(crate) conflicts: Vec<String>,
}

pub(crate) fn write_message<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(message).map_err(io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line)
}

pub(crate) fn parse_message<T: DeserializeOwned>(line: &str) -> serde_json::Result<T> {
    serde_json::from_str(line.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_messages_have_a_stable_wire_form() {
        let key = ToDaemon::Key {
            usage_page: 7,
            usage: 4,
            pressed: true,
            apple: true,
        };
        let mut line = Vec::new();
        write_message(&mut line, &key).unwrap();
        assert_eq!(
            String::from_utf8(line).unwrap(),
            "{\"type\":\"key\",\"usage_page\":7,\"usage\":4,\"pressed\":true,\"apple\":true}\n"
        );
    }

    #[test]
    fn every_message_round_trips() {
        let to_helper = [
            ToHelper::Hello {
                protocol: PROTOCOL_VERSION,
                role: Role::Session,
            },
            ToHelper::Hello {
                protocol: PROTOCOL_VERSION,
                role: Role::Status,
            },
            ToHelper::Heartbeat,
            ToHelper::Emit {
                usage_page: 0x0C,
                usage: 0xCD,
                pressed: false,
            },
            ToHelper::CapsLockLight { on: true },
        ];
        for message in to_helper {
            let mut line = Vec::new();
            write_message(&mut line, &message).unwrap();
            let parsed: ToHelper = parse_message(std::str::from_utf8(&line).unwrap()).unwrap();
            assert_eq!(parsed, message);
        }
        let to_daemon = [
            ToDaemon::Welcome { protocol: 1 },
            ToDaemon::Refused {
                protocol: 2,
                reason: "old daemon".to_string(),
            },
            ToDaemon::Seized { active: true },
            ToDaemon::Status(HelperStatus {
                protocol: 1,
                virtual_keyboard_ready: true,
                input_monitoring: false,
                seized: vec!["Apple Internal Keyboard / Trackpad".to_string()],
                conflicts: Vec::new(),
            }),
        ];
        for message in to_daemon {
            let mut line = Vec::new();
            write_message(&mut line, &message).unwrap();
            let parsed: ToDaemon = parse_message(std::str::from_utf8(&line).unwrap()).unwrap();
            assert_eq!(parsed, message);
        }
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(parse_message::<ToHelper>("{\"type\":\"nope\"}").is_err());
        assert!(parse_message::<ToHelper>("").is_err());
    }
}
