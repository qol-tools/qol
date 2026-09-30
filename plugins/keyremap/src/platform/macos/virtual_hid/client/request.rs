pub(crate) const CLIENT_PROTOCOL_VERSION: u16 = 7;
pub(crate) const VIRTUAL_KEYBOARD_VENDOR_ID: u64 = 0x16c0;
pub(crate) const VIRTUAL_KEYBOARD_PRODUCT_ID: u64 = 0x27db;

pub(crate) type Keys = [u16; 32];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    KeyboardInitialize { country_code: u64 },
    KeyboardReset,
    Keyboard { modifiers: u8, keys: Keys },
    Consumer(Keys),
    AppleVendorKeyboard(Keys),
    AppleVendorTopCase(Keys),
}

impl Request {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut bytes = CLIENT_PROTOCOL_VERSION.to_le_bytes().to_vec();
        match self {
            Self::KeyboardInitialize { country_code } => {
                bytes.push(0);
                for value in [
                    VIRTUAL_KEYBOARD_VENDOR_ID,
                    VIRTUAL_KEYBOARD_PRODUCT_ID,
                    *country_code,
                ] {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            Self::KeyboardReset => bytes.push(2),
            Self::Keyboard { modifiers, keys } => {
                bytes.extend_from_slice(&[6, 1, *modifiers, 0]);
                push_keys(&mut bytes, keys);
            }
            Self::Consumer(keys) => {
                bytes.extend_from_slice(&[7, 2]);
                push_keys(&mut bytes, keys);
            }
            Self::AppleVendorKeyboard(keys) => {
                bytes.extend_from_slice(&[8, 4]);
                push_keys(&mut bytes, keys);
            }
            Self::AppleVendorTopCase(keys) => {
                bytes.extend_from_slice(&[9, 3]);
                push_keys(&mut bytes, keys);
            }
        }
        bytes
    }
}

fn push_keys(bytes: &mut Vec<u8>, keys: &Keys) {
    for key in keys {
        bytes.extend_from_slice(&key.to_le_bytes());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    DriverActivated(bool),
    DriverConnected(bool),
    DriverVersionMismatched(bool),
    KeyboardReady(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusError {
    OddLength(usize),
    UnknownResponse(u8),
}

impl std::fmt::Display for StatusError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OddLength(length) => write!(formatter, "status message has odd length {length}"),
            Self::UnknownResponse(code) => write!(formatter, "unknown status code {code}"),
        }
    }
}

pub(crate) fn parse_status(payload: &[u8]) -> Result<Vec<Status>, StatusError> {
    if !payload.len().is_multiple_of(2) {
        return Err(StatusError::OddLength(payload.len()));
    }
    payload
        .as_chunks::<2>()
        .0
        .iter()
        .filter_map(|&[kind, value]| {
            let value = value != 0;
            match kind {
                0 | 5 => None,
                1 => Some(Ok(Status::DriverActivated(value))),
                2 => Some(Ok(Status::DriverConnected(value))),
                3 => Some(Ok(Status::DriverVersionMismatched(value))),
                4 => Some(Ok(Status::KeyboardReady(value))),
                other => Some(Err(StatusError::UnknownResponse(other))),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_initialize_sends_vendor_product_and_country_as_u64() {
        let bytes = Request::KeyboardInitialize { country_code: 0x0D }.encode();
        let mut expected = vec![7, 0, 0];
        expected.extend(0x16c0u64.to_le_bytes());
        expected.extend(0x27dbu64.to_le_bytes());
        expected.extend(0x0Du64.to_le_bytes());
        assert_eq!(bytes, expected);
    }

    #[test]
    fn keyboard_report_is_id_modifiers_reserved_then_32_keys() {
        let mut keys = [0u16; 32];
        keys[0] = 0x04;
        keys[31] = 0x0102;
        let bytes = Request::Keyboard {
            modifiers: 0x08,
            keys,
        }
        .encode();
        assert_eq!(bytes.len(), 3 + 67);
        assert_eq!(&bytes[..6], &[7, 0, 6, 1, 0x08, 0]);
        assert_eq!(&bytes[6..8], &[0x04, 0]);
        assert_eq!(&bytes[68..70], &[0x02, 0x01]);
    }

    #[test]
    fn page_reports_carry_their_report_ids() {
        let keys = [0u16; 32];
        assert_eq!(&Request::Consumer(keys).encode()[..4], &[7, 0, 7, 2]);
        assert_eq!(
            &Request::AppleVendorKeyboard(keys).encode()[..4],
            &[7, 0, 8, 4]
        );
        assert_eq!(
            &Request::AppleVendorTopCase(keys).encode()[..4],
            &[7, 0, 9, 3]
        );
        assert_eq!(Request::Consumer(keys).encode().len(), 3 + 65);
        assert_eq!(Request::KeyboardReset.encode(), vec![7, 0, 2]);
    }

    #[test]
    fn status_pairs_parse_and_none_is_skipped() {
        assert_eq!(
            parse_status(&[1, 1, 2, 0, 0, 0, 4, 1]),
            Ok(vec![
                Status::DriverActivated(true),
                Status::DriverConnected(false),
                Status::KeyboardReady(true),
            ])
        );
        assert_eq!(parse_status(&[]), Ok(Vec::new()));
        assert_eq!(parse_status(&[4]), Err(StatusError::OddLength(1)));
        assert_eq!(parse_status(&[9, 1]), Err(StatusError::UnknownResponse(9)));
    }
}
