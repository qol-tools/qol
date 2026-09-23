//! Platform window identity, normalized to one canonical string form.

/// Identity of an OS window, normalized to a canonical string form.
///
/// Two forms are recognized:
/// - X11-style numeric ids: `0x...` hex (lowercased) or bare decimal, which is
///   rewritten to hex. `u32` window handles (X11 window ids, macOS
///   `CGWindowNumber`) round-trip through [`WindowId::from_u32`] and
///   [`WindowId::as_u32`].
/// - macOS Accessibility ids: `pid:<n>`, stored verbatim.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WindowId(String);

impl WindowId {
    /// Build an id from a numeric window handle (X11 window id,
    /// `CGWindowNumber`).
    pub fn from_u32(id: u32) -> Self {
        Self(format!("0x{id:x}"))
    }

    /// Parse and normalize a platform window id string.
    ///
    /// Accepts `0x`-prefixed hex (case-insensitive), bare decimal, and
    /// `pid:<n>` forms; returns `None` for anything else.
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix("pid:") {
            return rest.parse::<u64>().ok().map(|_| Self(trimmed.to_string()));
        }
        if is_x11_hex(trimmed) {
            return Some(Self(trimmed.to_ascii_lowercase()));
        }
        let numeric = trimmed.parse::<u64>().ok()?;
        Some(Self(format!("0x{numeric:x}")))
    }

    /// The canonical string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The numeric window handle, when this id is X11-style (`0x...`).
    pub fn as_u32(&self) -> Option<u32> {
        self.0
            .strip_prefix("0x")
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
    }

    /// The process id, when this id is the macOS Accessibility `pid:<n>` form.
    pub fn as_pid(&self) -> Option<i64> {
        self.0
            .strip_prefix("pid:")
            .and_then(|rest| rest.parse().ok())
    }
}

fn is_x11_hex(id: &str) -> bool {
    let body = match id.get(0..2) {
        Some(prefix) if prefix.eq_ignore_ascii_case("0x") => &id[2..],
        _ => return false,
    };
    !body.is_empty() && body.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::WindowId;

    #[test]
    fn from_u32_round_trips_through_as_u32() {
        for id in [0, 1, 0x1234, 0x4c00004, u32::MAX] {
            let window_id = WindowId::from_u32(id);
            assert_eq!(window_id.as_u32(), Some(id), "id: {id:#x}");
        }
    }

    #[test]
    fn from_u32_uses_lowercase_hex() {
        assert_eq!(WindowId::from_u32(0x4c0_0004).as_str(), "0x4c00004");
        assert_eq!(WindowId::from_u32(10).as_str(), "0xa");
    }

    #[test]
    fn parse_normalizes_hex_and_decimal() {
        let cases = [
            ("0x123", Some("0x123")),
            ("0XABC", Some("0xabc")),
            ("0xDEADBEEF", Some("0xdeadbeef")),
            (" 0x1a ", Some("0x1a")),
            ("42", Some("0x2a")),
            ("0x", None),
            ("0xg1", None),
            ("hello", None),
            ("", None),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                WindowId::parse(raw).map(|id| id.as_str().to_string()),
                expected.map(String::from),
                "raw: {raw:?}"
            );
        }
    }

    #[test]
    fn parse_preserves_pid_form() {
        let window_id = WindowId::parse("pid:1234").expect("pid form parses");
        assert_eq!(window_id.as_str(), "pid:1234");
        assert_eq!(window_id.as_pid(), Some(1234));
        assert_eq!(window_id.as_u32(), None);
        assert!(WindowId::parse("pid:abc").is_none());
        assert!(WindowId::parse("pid:").is_none());
    }

    #[test]
    fn pid_ids_are_distinct_from_x11_ids() {
        let x11 = WindowId::parse("0x4d2").unwrap();
        let pid = WindowId::parse("pid:1234").unwrap();
        assert_ne!(x11, pid);
        assert_eq!(x11.as_pid(), None);
    }
}
