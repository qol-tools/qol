pub fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(hex.get(0..2)?, 16).ok()?;
    let g = u8::from_str_radix(hex.get(2..4)?, 16).ok()?;
    let b = u8::from_str_radix(hex.get(4..6)?, 16).ok()?;
    Some((r, g, b))
}

pub fn rgb24(red: u8, green: u8, blue: u8) -> u32 {
    ((red as u32) << 16) | ((green as u32) << 8) | (blue as u32)
}

pub fn scale_rgb(color: u32, brightness: f32) -> u32 {
    let brightness = clamp_unit(brightness);
    let red = scale_channel((color >> 16) & 0xff, brightness);
    let green = scale_channel((color >> 8) & 0xff, brightness);
    let blue = scale_channel(color & 0xff, brightness);
    (red << 16) | (green << 8) | blue
}

pub fn mix_rgb(color: u32, target: u32, amount: f32) -> u32 {
    let amount = clamp_unit(amount);
    let red = mix_channel((color >> 16) & 0xff, (target >> 16) & 0xff, amount);
    let green = mix_channel((color >> 8) & 0xff, (target >> 8) & 0xff, amount);
    let blue = mix_channel(color & 0xff, target & 0xff, amount);
    (red << 16) | (green << 8) | blue
}

pub fn rgba_from_rgb(color: u32, opacity: f32) -> u32 {
    let alpha = (clamp_unit(opacity) * 255.0).round() as u32;
    (color << 8) | alpha
}

pub const fn with_alpha(color: u32, alpha: u8) -> u32 {
    (color << 8) | alpha as u32
}

pub fn clamp_unit(value: f32) -> f32 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

fn scale_channel(value: u32, brightness: f32) -> u32 {
    (value as f32 * brightness).round() as u32
}

fn mix_channel(from: u32, to: u32, amount: f32) -> u32 {
    (from as f32 + (to as f32 - from as f32) * amount).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_color_accepts_hash_and_plain_rgb() {
        let cases = [
            ("#203040", Some((0x20, 0x30, 0x40))),
            ("203040", Some((0x20, 0x30, 0x40))),
            ("#ff8040", Some((0xff, 0x80, 0x40))),
            ("#FF8040", Some((0xff, 0x80, 0x40))),
            ("#123", None),
            ("#12345678", None),
            ("nope", None),
            ("aäxyz", None),
            ("#aäxyz", None),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_hex_color(input), expected, "input: {input}");
        }
    }

    #[test]
    fn rgb24_packs_channels() {
        assert_eq!(rgb24(0x20, 0x30, 0x40), 0x203040);
    }

    #[test]
    fn scale_rgb_clamps_brightness() {
        let cases = [
            (0x102030, 1.0, 0x102030),
            (0xff8040, 0.25, 0x402010),
            (0x102030, 2.0, 0x102030),
            (0x102030, -1.0, 0x000000),
            (0x102030, f32::NAN, 0x000000),
        ];
        for (color, brightness, expected) in cases {
            assert_eq!(scale_rgb(color, brightness), expected);
        }
    }

    #[test]
    fn mix_rgb_clamps_amount() {
        let cases = [
            (0x000000, 0xffffff, 0.0, 0x000000),
            (0x000000, 0xffffff, 0.5, 0x808080),
            (0x000000, 0xffffff, 1.0, 0xffffff),
            (0x102030, 0xffffff, 2.0, 0xffffff),
            (0x102030, 0xffffff, -1.0, 0x102030),
            (0x102030, 0xffffff, f32::NAN, 0x102030),
        ];
        for (color, target, amount, expected) in cases {
            assert_eq!(mix_rgb(color, target, amount), expected);
        }
    }

    #[test]
    fn rgba_from_rgb_clamps_opacity() {
        let cases = [
            (0x203040, 0.0, 0x20304000),
            (0x203040, 0.5, 0x20304080),
            (0x203040, 1.0, 0x203040ff),
            (0x203040, 2.0, 0x203040ff),
            (0x203040, -1.0, 0x20304000),
            (0x203040, f32::NAN, 0x20304000),
        ];
        for (color, opacity, expected) in cases {
            assert_eq!(rgba_from_rgb(color, opacity), expected);
        }
    }

    #[test]
    fn with_alpha_appends_exact_alpha_byte() {
        let cases = [
            (0x203040, 0x00, 0x20304000),
            (0x203040, 0x22, 0x20304022),
            (0x203040, 0xff, 0x203040ff),
        ];
        for (color, alpha, expected) in cases {
            assert_eq!(with_alpha(color, alpha), expected, "alpha: {alpha:#04x}");
        }
    }
}
