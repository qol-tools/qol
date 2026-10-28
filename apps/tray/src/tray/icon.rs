use super::platform::{ICON_SIDE_PADDING, ICON_SIZE};
use super::tone::{PanelLook, PanelTone};
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

const DRAWN_ACCENT: (u8, u8, u8) = (0xff, 0x6b, 0x6b);

pub fn tray_icon(update: bool, look: PanelLook) -> Icon {
    let mut pixels = art(update, look.tone).to_vec();
    if let Some(accent) = look.update_accent.filter(|_| update) {
        recolor_accent(&mut pixels, art(false, look.tone), accent);
    }
    let canvas = pad_sides(&pixels, ICON_SIZE, ICON_SIDE_PADDING);
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

fn recolor_accent(art: &mut [u8], idle: &[u8], accent: (u8, u8, u8)) {
    let (drawn_r, drawn_g, drawn_b) = DRAWN_ACCENT;
    let span = f32::from(drawn_r - drawn_g.max(drawn_b));
    let shift = [
        f32::from(accent.0) - f32::from(drawn_r),
        f32::from(accent.1) - f32::from(drawn_g),
        f32::from(accent.2) - f32::from(drawn_b),
    ];
    for (pixel, idle) in art
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(idle.as_chunks::<4>().0)
    {
        if pixel == idle {
            continue;
        }
        let excess = f32::from(pixel[0]) - f32::from(pixel[1].max(pixel[2]));
        let weight = (excess / span).clamp(0.0, 1.0);
        for (channel, delta) in pixel[..3].iter_mut().zip(shift) {
            *channel = (f32::from(*channel) + weight * delta)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
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

    const OTHER_ACCENT: (u8, u8, u8) = (0x00, 0xff, 0x00);

    fn drawn_accent_channels() -> [u8; 3] {
        [DRAWN_ACCENT.0, DRAWN_ACCENT.1, DRAWN_ACCENT.2]
    }

    #[test]
    fn tray_icon_decodes_every_variant() {
        for tone in [PanelTone::Dark, PanelTone::Light] {
            for update in [false, true] {
                for update_accent in [None, Some(OTHER_ACCENT)] {
                    let _icon = tray_icon(
                        update,
                        PanelLook {
                            tone,
                            update_accent,
                        },
                    );
                }
            }
        }
    }

    #[test]
    fn recolor_to_the_drawn_accent_leaves_the_art_unchanged() {
        for tone in [PanelTone::Dark, PanelTone::Light] {
            for update in [false, true] {
                let mut recolored = art(update, tone).to_vec();
                recolor_accent(&mut recolored, art(false, tone), DRAWN_ACCENT);
                assert_eq!(recolored, art(update, tone), "{tone:?} update={update}");
            }
        }
    }

    #[test]
    fn recolor_swaps_the_drawn_accent_over_a_neutral_base_and_keeps_alpha() {
        let drawn_accent = drawn_accent_channels().map(f32::from);
        let accent = [OTHER_ACCENT.0, OTHER_ACCENT.1, OTHER_ACCENT.2].map(f32::from);
        let span = drawn_accent[0] - drawn_accent[1].max(drawn_accent[2]);
        for tone in [PanelTone::Dark, PanelTone::Light] {
            let (idle, drawn) = (art(false, tone), art(true, tone));
            let mut recolored = drawn.to_vec();
            recolor_accent(&mut recolored, idle, OTHER_ACCENT);
            let mut blended = 0;
            for ((before, after), idle) in drawn
                .as_chunks::<4>()
                .0
                .iter()
                .zip(recolored.as_chunks::<4>().0.iter())
                .zip(idle.as_chunks::<4>().0.iter())
            {
                assert_eq!(before[3], after[3], "{tone:?} alpha of {before:?}");
                if before == idle {
                    assert_eq!(before, after, "{tone:?} idle pixel");
                    continue;
                }
                let channels = [before[0], before[1], before[2]].map(f32::from);
                let weight = ((channels[0] - channels[1].max(channels[2])) / span).clamp(0.0, 1.0);
                if weight > 0.0 && weight < 1.0 {
                    blended += 1;
                }
                for (((&value, &drawn_value), &accent_value), &actual) in channels
                    .iter()
                    .zip(&drawn_accent)
                    .zip(&accent)
                    .zip(&after[..3])
                {
                    let base = if weight < 1.0 {
                        (value - weight * drawn_value) / (1.0 - weight)
                    } else {
                        0.0
                    };
                    let expected =
                        (weight * accent_value + (1.0 - weight) * base).clamp(0.0, 255.0);
                    assert!(
                        (f32::from(actual) - expected).abs() <= 0.5,
                        "{tone:?} {before:?} -> {after:?}: {actual} vs {expected}"
                    );
                }
            }
            assert!(blended > 0, "{tone:?} has no antialiased nut edge");
        }
    }

    #[test]
    fn recolor_paints_the_solid_nut_in_the_accent() {
        for tone in [PanelTone::Dark, PanelTone::Light] {
            let drawn = art(true, tone);
            let mut recolored = drawn.to_vec();
            recolor_accent(&mut recolored, art(false, tone), OTHER_ACCENT);
            let solid = drawn
                .as_chunks::<4>()
                .0
                .iter()
                .zip(recolored.as_chunks::<4>().0.iter())
                .filter(|(before, _)| before[..3] == drawn_accent_channels())
                .map(|(_, after)| after)
                .collect::<Vec<_>>();
            assert!(!solid.is_empty(), "{tone:?} has no solid nut pixels");
            for pixel in solid {
                assert_eq!(
                    pixel[..3],
                    [OTHER_ACCENT.0, OTHER_ACCENT.1, OTHER_ACCENT.2],
                    "{tone:?}"
                );
            }
        }
    }
}
