use super::platform::{ICON_SIDE_PADDING, ICON_SIZE};
use super::tone::PanelTone;
use tray_icon::Icon;

struct ArtSet {
    dark_idle: &'static [u8],
    dark_update: &'static [u8],
    light_idle: &'static [u8],
    light_update: &'static [u8],
}

macro_rules! art_file {
    ($name:literal, $size:literal) => {
        checked(
            include_bytes!(concat!("../../assets/tray/", $name, "-", $size, ".rgba")),
            $size,
        )
    };
}

macro_rules! art_set {
    ($size:literal) => {
        ArtSet {
            dark_idle: art_file!("dark-idle", $size),
            dark_update: art_file!("dark-update", $size),
            light_idle: art_file!("light-idle", $size),
            light_update: art_file!("light-update", $size),
        }
    };
}

const ART: ArtSet = match ICON_SIZE {
    32 => art_set!(32),
    64 => art_set!(64),
    _ => panic!("no tray art for this icon size"),
};

const fn checked(bytes: &'static [u8], size: u32) -> &'static [u8] {
    assert!(
        bytes.len() == (size * size * 4) as usize,
        "tray art must be square RGBA of its size"
    );
    bytes
}

pub fn tray_icon(update: bool, tone: PanelTone) -> Icon {
    let canvas = pad_sides(art(update, tone), ICON_SIZE, ICON_SIDE_PADDING);
    Icon::from_rgba(canvas, ICON_SIZE + 2 * ICON_SIDE_PADDING, ICON_SIZE)
        .expect("embedded tray art is valid")
}

fn pad_sides(art: &[u8], size: u32, padding: u32) -> Vec<u8> {
    let art_row = (size * 4) as usize;
    let canvas_row = ((size + 2 * padding) * 4) as usize;
    let left = (padding * 4) as usize;
    let mut canvas = vec![0u8; canvas_row * size as usize];
    for (src, dst) in art
        .chunks_exact(art_row)
        .zip(canvas.chunks_exact_mut(canvas_row))
    {
        dst[left..left + art_row].copy_from_slice(src);
    }
    canvas
}

fn art(update: bool, tone: PanelTone) -> &'static [u8] {
    match (tone, update) {
        (PanelTone::Dark, false) => ART.dark_idle,
        (PanelTone::Dark, true) => ART.dark_update,
        (PanelTone::Light, false) => ART.light_idle,
        (PanelTone::Light, true) => ART.light_update,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_is_distinct_art_of_the_platform_size() {
        let variants = [
            art(false, PanelTone::Dark),
            art(true, PanelTone::Dark),
            art(false, PanelTone::Light),
            art(true, PanelTone::Light),
        ];
        for (index, bytes) in variants.iter().enumerate() {
            assert_eq!(bytes.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);
            for other in &variants[index + 1..] {
                assert_ne!(bytes, other, "variant {index} repeats another variant");
            }
        }
    }

    #[test]
    fn pad_sides_without_padding_returns_the_art_unchanged() {
        let art: Vec<u8> = (1..=2 * 2 * 4).collect();
        assert_eq!(pad_sides(&art, 2, 0), art);
    }

    #[test]
    fn pad_sides_centres_each_row_between_transparent_margins() {
        let art: Vec<u8> = (1..=2 * 2 * 4).collect();
        let canvas = pad_sides(&art, 2, 3);
        let canvas_row = (2 + 2 * 3) * 4;
        assert_eq!(canvas.len(), canvas_row * 2);
        for (y, row) in canvas.chunks_exact(canvas_row).enumerate() {
            assert!(row[..12].iter().all(|&b| b == 0), "left margin of row {y}");
            assert_eq!(&row[12..20], &art[y * 8..y * 8 + 8], "art row {y}");
            assert!(row[20..].iter().all(|&b| b == 0), "right margin of row {y}");
        }
    }

    #[test]
    fn tray_icon_decodes_every_variant() {
        for tone in [PanelTone::Dark, PanelTone::Light] {
            for update in [false, true] {
                let _icon = tray_icon(update, tone);
            }
        }
    }
}
