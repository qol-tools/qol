use sha2::{Digest, Sha256};

use qol_windowing::display::{DisplayMode, DisplayPlacement};

const DEVICE_PREFIX: &str = r"\\.\";
const INTERFACE_PREFIX: &str = r"\\?\";
const BASE_EDID_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplaySetting {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub bits_per_pixel: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchoredPlacement {
    pub connector: String,
    pub x: i32,
    pub y: i32,
    pub primary: bool,
}

pub fn mode_token(width: u32, height: u32, refresh_hz: u32) -> u64 {
    (u64::from(width) << 32) | (u64::from(height & 0xffff) << 16) | u64::from(refresh_hz & 0xffff)
}

pub fn mode_from_setting(setting: DisplaySetting) -> DisplayMode {
    DisplayMode {
        token: mode_token(setting.width, setting.height, setting.refresh_hz),
        width: setting.width,
        height: setting.height,
        refresh_hz: setting.refresh_hz,
    }
}

pub fn distinct_modes(settings: &[DisplaySetting]) -> Vec<DisplayMode> {
    let mut modes: Vec<DisplayMode> = Vec::new();
    for setting in settings {
        let mode = mode_from_setting(*setting);
        if !modes.iter().any(|known| known.token == mode.token) {
            modes.push(mode);
        }
    }
    modes.sort_by(|a, b| {
        (u64::from(b.width) * u64::from(b.height))
            .cmp(&(u64::from(a.width) * u64::from(a.height)))
            .then(b.width.cmp(&a.width))
            .then(b.refresh_hz.cmp(&a.refresh_hz))
    });
    modes
}

pub fn pick_setting(
    settings: &[DisplaySetting],
    mode: &DisplayMode,
    current_bits: u32,
) -> Option<DisplaySetting> {
    let matching = settings.iter().filter(|setting| {
        setting.width == mode.width
            && setting.height == mode.height
            && setting.refresh_hz == mode.refresh_hz
    });
    matching
        .clone()
        .find(|setting| setting.bits_per_pixel == current_bits)
        .or_else(|| matching.max_by_key(|setting| setting.bits_per_pixel))
        .copied()
}

pub fn primary_anchored(placements: &[DisplayPlacement]) -> Vec<AnchoredPlacement> {
    let (shift_x, shift_y) = placements
        .iter()
        .find(|placement| placement.primary)
        .map_or((0, 0), |primary| (primary.x, primary.y));
    placements
        .iter()
        .map(|placement| AnchoredPlacement {
            connector: placement.handle.connector().to_string(),
            x: placement.x - shift_x,
            y: placement.y - shift_y,
            primary: placement.primary,
        })
        .collect()
}

pub fn connector_from_device(device: &str) -> &str {
    device.strip_prefix(DEVICE_PREFIX).unwrap_or(device)
}

pub fn device_from_connector(connector: &str) -> String {
    format!("{DEVICE_PREFIX}{}", connector_from_device(connector))
}

pub fn device_instance_from_interface(interface: &str) -> Option<String> {
    let path = interface.strip_prefix(INTERFACE_PREFIX)?;
    let mut parts = path.split('#');
    let enumerator = parts.next().filter(|part| !part.is_empty())?;
    let hardware = parts.next().filter(|part| !part.is_empty())?;
    let instance = parts.next().filter(|part| !part.is_empty())?;
    parts.next()?;
    Some(format!(r"{enumerator}\{hardware}\{instance}"))
}

pub fn edid_registry_key(instance: &str) -> String {
    format!(r"SYSTEM\CurrentControlSet\Enum\{instance}\Device Parameters")
}

pub fn identity_from(
    instance: Option<&str>,
    connector: &str,
    edid: Option<&[u8]>,
) -> (String, Option<[u8; 32]>, bool) {
    let binding = instance.unwrap_or(connector);
    match edid.filter(|edid| !edid.is_empty()) {
        Some(edid) => {
            let base = &edid[..edid.len().min(BASE_EDID_BYTES)];
            let digest: [u8; 32] = Sha256::digest(base).into();
            let mut hasher = Sha256::new();
            hasher.update(binding.as_bytes());
            hasher.update(base);
            let bound: [u8; 32] = hasher.finalize().into();
            (hex(&bound), Some(digest), false)
        }
        None => (hex(&Sha256::digest(binding.as_bytes()).into()), None, true),
    }
}

pub fn display_change_reason(code: i32) -> &'static str {
    match code {
        1 => "the change needs a restart",
        -1 => "the display driver failed the mode",
        -2 => "the graphics mode is not supported",
        -3 => "the settings could not be written to the registry",
        -4 => "an invalid set of flags was passed",
        -5 => "an invalid parameter was passed",
        -6 => "the system is DualView capable",
        _ => "the display change failed",
    }
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_windowing::display::DisplayHandle;

    fn setting(width: u32, height: u32, refresh_hz: u32, bits_per_pixel: u32) -> DisplaySetting {
        DisplaySetting {
            width,
            height,
            refresh_hz,
            bits_per_pixel,
        }
    }

    fn placement(connector: &str, x: i32, y: i32, primary: bool) -> DisplayPlacement {
        DisplayPlacement {
            handle: DisplayHandle::new(connector.into(), connector.into(), None, false),
            x,
            y,
            primary,
        }
    }

    #[test]
    fn mode_token_packs_width_height_and_refresh() {
        let cases = [
            (1920, 1080, 60, 0x0000_0780_0438_003c),
            (2560, 1440, 144, 0x0000_0a00_05a0_0090),
            (3840, 2160, 120, 0x0000_0f00_0870_0078),
            (0, 0, 0, 0),
        ];
        for (width, height, refresh, expected) in cases {
            assert_eq!(
                mode_token(width, height, refresh),
                expected,
                "{width}x{height}@{refresh}"
            );
        }
    }

    #[test]
    fn distinct_modes_dedupes_bit_depths_and_sorts_largest_first() {
        let settings = [
            setting(1280, 720, 60, 32),
            setting(1920, 1080, 60, 8),
            setting(1920, 1080, 60, 32),
            setting(1920, 1080, 144, 32),
            setting(1680, 1050, 60, 32),
            setting(1280, 720, 60, 16),
        ];
        let described = distinct_modes(&settings)
            .iter()
            .map(|mode| {
                assert_eq!(
                    mode.token,
                    mode_token(mode.width, mode.height, mode.refresh_hz)
                );
                format!("{}x{}@{}", mode.width, mode.height, mode.refresh_hz)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            described,
            vec![
                "1920x1080@144",
                "1920x1080@60",
                "1680x1050@60",
                "1280x720@60"
            ]
        );
        assert!(distinct_modes(&[]).is_empty());
    }

    #[test]
    fn pick_setting_prefers_the_current_bit_depth_then_the_deepest() {
        let settings = [
            setting(1920, 1080, 60, 8),
            setting(1920, 1080, 60, 16),
            setting(1920, 1080, 60, 32),
            setting(1920, 1080, 144, 16),
        ];
        let mode = |width, height, refresh_hz| DisplayMode {
            token: mode_token(width, height, refresh_hz),
            width,
            height,
            refresh_hz,
        };
        let cases = [
            (mode(1920, 1080, 60), 16, Some(16)),
            (mode(1920, 1080, 60), 24, Some(32)),
            (mode(1920, 1080, 144), 32, Some(16)),
            (mode(1920, 1080, 75), 32, None),
            (mode(1280, 720, 60), 32, None),
        ];
        for (requested, current_bits, expected) in cases {
            let picked = pick_setting(&settings, &requested, current_bits);
            assert_eq!(
                picked.map(|setting| setting.bits_per_pixel),
                expected,
                "{}x{}@{} at {current_bits} bpp",
                requested.width,
                requested.height,
                requested.refresh_hz
            );
            if let Some(picked) = picked {
                assert_eq!(
                    (picked.width, picked.height, picked.refresh_hz),
                    (requested.width, requested.height, requested.refresh_hz)
                );
            }
        }
    }

    #[test]
    fn primary_anchored_moves_the_primary_to_the_origin() {
        let cases = [
            (
                vec![
                    placement("DISPLAY1", 0, 0, true),
                    placement("DISPLAY2", 1920, 0, false),
                ],
                vec![("DISPLAY1", 0, 0, true), ("DISPLAY2", 1920, 0, false)],
            ),
            (
                vec![
                    placement("DISPLAY1", 0, 0, false),
                    placement("DISPLAY2", 2560, 0, true),
                ],
                vec![("DISPLAY1", -2560, 0, false), ("DISPLAY2", 0, 0, true)],
            ),
            (
                vec![
                    placement("DISPLAY1", 100, 300, true),
                    placement("DISPLAY2", 100, -780, false),
                    placement("DISPLAY3", 2020, 300, false),
                ],
                vec![
                    ("DISPLAY1", 0, 0, true),
                    ("DISPLAY2", 0, -1080, false),
                    ("DISPLAY3", 1920, 0, false),
                ],
            ),
            (
                vec![placement("DISPLAY1", 40, 40, false)],
                vec![("DISPLAY1", 40, 40, false)],
            ),
        ];
        for (placements, expected) in cases {
            let anchored = primary_anchored(&placements)
                .into_iter()
                .map(|placed| (placed.connector, placed.x, placed.y, placed.primary))
                .collect::<Vec<_>>();
            let expected = expected
                .into_iter()
                .map(|(connector, x, y, primary)| (connector.to_string(), x, y, primary))
                .collect::<Vec<_>>();
            assert_eq!(anchored, expected);
        }
    }

    #[test]
    fn connector_and_device_names_round_trip() {
        let cases = [
            (r"\\.\DISPLAY1", "DISPLAY1", r"\\.\DISPLAY1"),
            (r"\\.\DISPLAY12", "DISPLAY12", r"\\.\DISPLAY12"),
            ("DISPLAY3", "DISPLAY3", r"\\.\DISPLAY3"),
        ];
        for (device, connector, back) in cases {
            assert_eq!(connector_from_device(device), connector);
            assert_eq!(device_from_connector(connector), back);
            assert_eq!(device_from_connector(device), back);
        }
    }

    #[test]
    fn device_instance_parses_monitor_interface_paths() {
        let cases = [
            (
                r"\\?\DISPLAY#DEL4105#5&2d6b5c2&0&UID4353#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}",
                Some(r"DISPLAY\DEL4105\5&2d6b5c2&0&UID4353"),
            ),
            (
                r"\\?\DISPLAY#Default_Monitor#1&8713bca&0&UID0#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}",
                Some(r"DISPLAY\Default_Monitor\1&8713bca&0&UID0"),
            ),
            (
                r"MONITOR\DEL4105\{4d36e96e-e325-11ce-bfc1-08002be10318}\0003",
                None,
            ),
            (r"\\?\DISPLAY#DEL4105", None),
            (r"\\?\DISPLAY##5&2d6b5c2#{guid}", None),
            ("", None),
        ];
        for (interface, expected) in cases {
            assert_eq!(
                device_instance_from_interface(interface).as_deref(),
                expected,
                "{interface}"
            );
        }
        assert_eq!(
            edid_registry_key(r"DISPLAY\DEL4105\5&2d6b5c2&0&UID4353"),
            r"SYSTEM\CurrentControlSet\Enum\DISPLAY\DEL4105\5&2d6b5c2&0&UID4353\Device Parameters"
        );
    }

    #[test]
    fn identity_binds_the_instance_and_the_base_edid() {
        let edid = [[0x11u8; 128], [0x22u8; 128]].concat();
        let instance = r"DISPLAY\DEL4105\5&2d6b5c2&0&UID4353";
        let (id, digest, unstable) = identity_from(Some(instance), "DISPLAY1", Some(&edid));
        assert!(!unstable);
        let base_digest: [u8; 32] = Sha256::digest([0x11u8; 128]).into();
        assert_eq!(digest, Some(base_digest));
        let mut hasher = Sha256::new();
        hasher.update(instance.as_bytes());
        hasher.update([0x11u8; 128]);
        let bound: [u8; 32] = hasher.finalize().into();
        assert_eq!(id, hex(&bound));

        let cases = [
            (
                Some(instance),
                "DISPLAY2",
                Some(&edid[..128]),
                id.clone(),
                false,
            ),
            (
                Some("DISPLAY\\DEL4105\\other"),
                "DISPLAY1",
                Some(&edid[..]),
                String::new(),
                false,
            ),
            (
                Some(instance),
                "DISPLAY1",
                None,
                hex(&Sha256::digest(instance.as_bytes()).into()),
                true,
            ),
            (
                None,
                "DISPLAY1",
                None,
                hex(&Sha256::digest(b"DISPLAY1").into()),
                true,
            ),
            (
                None,
                "DISPLAY1",
                Some(&[][..]),
                hex(&Sha256::digest(b"DISPLAY1").into()),
                true,
            ),
        ];
        for (instance, connector, edid, expected, expected_unstable) in cases {
            let (actual, _, actual_unstable) = identity_from(instance, connector, edid);
            assert_eq!(
                actual_unstable, expected_unstable,
                "{instance:?} {connector}"
            );
            if expected.is_empty() {
                assert_ne!(actual, id, "a different instance must change the id");
            } else {
                assert_eq!(actual, expected, "{instance:?} {connector}");
            }
        }
    }

    #[test]
    fn display_change_reasons_name_every_documented_code() {
        let cases = [
            (1, "the change needs a restart"),
            (-1, "the display driver failed the mode"),
            (-2, "the graphics mode is not supported"),
            (-3, "the settings could not be written to the registry"),
            (-4, "an invalid set of flags was passed"),
            (-5, "an invalid parameter was passed"),
            (-6, "the system is DualView capable"),
            (-99, "the display change failed"),
        ];
        for (code, expected) in cases {
            assert_eq!(display_change_reason(code), expected, "code {code}");
        }
    }
}
