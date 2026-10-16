use super::platform::ICON_SIZE;
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
    Icon::from_rgba(art(update, tone).to_vec(), ICON_SIZE, ICON_SIZE)
        .expect("embedded tray art is valid")
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
    fn tray_icon_decodes_every_variant() {
        for tone in [PanelTone::Dark, PanelTone::Light] {
            for update in [false, true] {
                let _icon = tray_icon(update, tone);
            }
        }
    }
}
