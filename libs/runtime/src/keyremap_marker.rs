#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyRemapMarker {
    pub mods: u8,
    pub key: u16,
}

pub const MOD_CTRL: u8 = 1 << 0;
pub const MOD_SHIFT: u8 = 1 << 1;
pub const MOD_ALT: u8 = 1 << 2;
pub const MOD_SUPER: u8 = 1 << 3;

const MAGIC: u64 = 0x514f_4b52_0000_0000;
const MAGIC_MASK: u64 = 0xffff_ffff_0000_0000;

pub fn encode(mods: u8, key: u16) -> i64 {
    (MAGIC | ((mods as u64) << 16) | key as u64) as i64
}

pub fn decode(value: i64) -> Option<KeyRemapMarker> {
    let raw = value as u64;
    if raw & MAGIC_MASK != MAGIC {
        return None;
    }
    Some(KeyRemapMarker {
        mods: ((raw >> 16) & 0xff) as u8,
        key: (raw & 0xffff) as u16,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_round_trips_mods_and_key() {
        let mods = MOD_CTRL | MOD_SHIFT | MOD_SUPER;
        let marker = decode(encode(mods, 0x0f)).expect("marker");
        assert_eq!(marker.mods, mods);
        assert_eq!(marker.key, 0x0f);
    }

    #[test]
    fn decode_rejects_unmarked_values() {
        assert_eq!(decode(0), None);
        assert_eq!(decode(0x1234), None);
    }
}
