use std::path::PathBuf;

use qol_gpui::pictures::{
    chevron, fitted_image, image, letters_for, markup, stacked_image, tick, PictureContext, Tone,
};
use qol_theme::{DesktopThemePreview, ThemeMode, WebThemePreview};

const BONE: DesktopThemePreview = DesktopThemePreview {
    pane: 0xfffefb,
    rail: 0xf5f2eb,
    edge: 0xe2ded4,
    ink: 0x1a1815,
    soft: 0x78736a,
    band: 0xbbb2d1,
    accent: 0x6f5da8,
};

const SLATE: DesktopThemePreview = DesktopThemePreview {
    pane: 0x16171a,
    rail: 0x101114,
    edge: 0x2a2b31,
    ink: 0xf3f2f0,
    soft: 0x8b8880,
    band: 0x464a79,
    accent: 0x8a93f7,
};

const WEB_SLATE: WebThemePreview = WebThemePreview {
    bg: 0x111317,
    surface: 0x1a1e26,
    raised: 0x272d38,
    border: 0x3e485b,
    text: 0x8a97ae,
    muted: 0x55627a,
};

const WEB_MIDNIGHT: WebThemePreview = WebThemePreview {
    bg: 0x090b19,
    surface: 0x141626,
    raised: 0x1b1e30,
    border: 0x34374d,
    text: 0xb2b6cc,
    muted: 0x555974,
};

const GOLDENS: &[&str] = &[
    "hold-to-switch",
    "sticky",
    "cycle-once",
    "show-only",
    "icon-corner:right",
    "icon-corner:left",
    "aa:15",
    "aa:22",
    "aa:30",
    "desktop-theme:bone",
    "desktop-theme:slate",
    "web-theme:slate",
    "web-theme:midnight",
    "swatch:amber",
    "swatch:green",
    "swatch:cyan",
    "swatch:magenta",
    "swatch:blue",
    "swatch:violet",
    "qol-toasts",
    "system-bubbles",
    "both-notices",
    "launch-app",
    "open-url",
    "bundle-ref",
    "path-ref",
    "name-ref",
    "browser-bundle-ref",
    "browser-path-ref",
    "browser-name-ref",
    "letters:D",
    "letters:WL",
    "letters:AT",
    "letters:B",
    "letters:CS",
    "letters:C",
    "letters:IC",
    "letters:KR",
    "letters:L",
    "letters:Li",
    "letters:M",
    "letters:OT",
    "letters:PZ",
    "letters:QM",
    "letters:QS",
    "letters:QV",
    "letters:WA",
    "letters:ZT",
    "letters:PK",
    "letters:W",
    "letters:SV",
    "letters:PF",
    "letters:NC",
    "letters:MS",
    "next-window",
    "previous-window",
    "gear",
    "adapter-auto",
    "adapter-chip",
    "schedule-off",
    "schedule-daily",
    "mic-default",
    "usb-mic",
    "webcam",
    "headset",
    "speaker-default",
    "hdmi",
    "headphones",
    "preset:100,100",
    "preset:88,92",
    "preset:76,84",
    "preset:64,78",
    "preset:54,72",
    "preset:44,66",
    "preset:33,60",
    "preset:22,56",
    "preset:12,52",
    "format:MKV",
    "format:MP4",
    "format:MOV",
    "format:WEBM",
    "copy-image",
    "copy-path",
    "no-terminal",
    "terminal-session:claude",
    "insert-only",
    "insert-submit",
    "prefer-local",
    "local-engine:onnx",
    "local-engine:candle",
    "remote-engine",
    "no-model",
    "model",
    "family-auto",
    "fixed-size",
    "relative-size",
];

const PICKER_LABELS: &[&str] = &[
    "Alt Tab",
    "Bluetooth",
    "CLI Sessions",
    "Controllers",
    "IDE Checkout",
    "Key Remap",
    "Launcher",
    "Lights",
    "Monitor",
    "OS Themes",
    "PointZerver",
    "QoL Memory",
    "QoL Shot",
    "QoL Voice",
    "Window Actions",
];

const PICKER_LETTERS: &[&str] = &[
    "AT", "B", "CS", "C", "IC", "KR", "L", "Li", "M", "OT", "PZ", "QM", "QS", "QV", "WA",
];

fn context() -> PictureContext {
    PictureContext {
        mode: ThemeMode::Dark,
        bone: BONE,
        slate: SLATE,
        web_slate: WEB_SLATE,
        web_midnight: WEB_MIDNIGHT,
    }
}

fn golden_path(spec: &str) -> PathBuf {
    let name = spec.replace([':', ','], "-");
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pictures")
        .join(format!("{name}.svg"))
}

#[test]
fn golden_markup_matches_the_locked_pictures() {
    let context = context();
    for spec in GOLDENS {
        let path = golden_path(spec);
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let actual = markup(spec, &context).unwrap_or_else(|| panic!("markup {spec}"));
        assert_eq!(actual, expected, "{spec}");
    }
}

#[test]
fn for_accent_reads_the_violet_preview() {
    let context = PictureContext::for_accent(ThemeMode::Dark, "violet");
    assert_eq!(context.slate, SLATE);
}

#[test]
fn every_picture_name_renders() {
    let context = context();
    for name in qol_config::contract::PICTURE_NAMES {
        let spec = sample_spec(name);
        let rendered =
            image(&spec, SLATE.ink, 224, 140, &context).unwrap_or_else(|| panic!("image {spec}"));
        let bytes = rendered
            .as_bytes(0)
            .unwrap_or_else(|| panic!("frame {spec}"));
        assert!(bytes.chunks(4).any(|pixel| pixel[3] != 0), "{spec}");
    }
}

#[test]
fn every_picture_name_fits_whole_in_both_tones() {
    let context = context();
    for name in qol_config::contract::PICTURE_NAMES {
        let spec = sample_spec(name);
        for tone in [Tone::Rest, Tone::Awake] {
            for scale in [1.0f32, 2.0] {
                let rendered = fitted_image(&spec, SLATE.soft, tone, 56.0, 35.0, scale, &context)
                    .unwrap_or_else(|| panic!("fitted image {spec}"));
                let bytes = rendered
                    .as_bytes(0)
                    .unwrap_or_else(|| panic!("frame {spec}"));
                assert!(bytes.chunks(4).any(|pixel| pixel[3] != 0), "{spec}");
            }
        }
    }
}

#[test]
fn display_modes_draw_a_screen_in_their_own_shape() {
    let context = context();
    let wide = markup("display-mode:2560x1440", &context).unwrap_or_else(|| panic!("wide mode"));
    assert!(wide.contains("<rect x=\"10\" y=\"5.63\" width=\"76\" height=\"42.75\" rx=\"4\"/>"));
    let tall = markup("display-mode:1280x1024", &context).unwrap_or_else(|| panic!("tall mode"));
    assert!(tall.contains("<rect x=\"20.5\" y=\"5\" width=\"55\" height=\"44\" rx=\"4\"/>"));
    assert!(tall.contains("<line x1=\"48\" y1=\"49\" x2=\"48\" y2=\"55\"/>"));
    assert!(tall.contains("<line x1=\"40\" y1=\"55\" x2=\"56\" y2=\"55\"/>"));
}

#[test]
fn the_stack_and_the_empty_tile_render_in_both_tones() {
    let context = context();
    for tone in [Tone::Rest, Tone::Awake] {
        for scale in [1.0f32, 2.0] {
            let width_px = (56.0 * scale).round() as u32;
            let height_px = (35.0 * scale).round() as u32;
            let frames = [
                stacked_image(
                    "letters:WH",
                    "letters:PB",
                    SLATE.ink,
                    tone,
                    56.0,
                    35.0,
                    scale,
                    &context,
                ),
                stacked_image(
                    "mic-default",
                    "speaker-default",
                    SLATE.ink,
                    tone,
                    56.0,
                    35.0,
                    scale,
                    &context,
                ),
                fitted_image("empty", SLATE.ink, tone, 56.0, 35.0, scale, &context),
            ];
            for frame in frames {
                let rendered = frame.unwrap_or_else(|| panic!("render {tone:?} {scale}"));
                let bytes = rendered
                    .as_bytes(0)
                    .unwrap_or_else(|| panic!("frame {tone:?} {scale}"));
                assert_eq!(bytes.len(), (width_px * height_px * 4) as usize);
                assert!(
                    bytes.chunks(4).any(|pixel| pixel[3] != 0),
                    "{tone:?} {scale}"
                );
            }
        }
    }
}

#[test]
fn a_colour_choice_keeps_its_colour_at_rest() {
    let context = context();
    for scale in [1.0f32, 2.0] {
        let rest = fitted_image(
            "swatch:violet",
            SLATE.soft,
            Tone::Rest,
            56.0,
            35.0,
            scale,
            &context,
        )
        .unwrap_or_else(|| panic!("swatch rest {scale}"));
        let awake = fitted_image(
            "swatch:violet",
            SLATE.soft,
            Tone::Awake,
            56.0,
            35.0,
            scale,
            &context,
        )
        .unwrap_or_else(|| panic!("swatch awake {scale}"));
        let rest_bytes = rest.as_bytes(0).unwrap_or_else(|| panic!("frame {scale}"));
        let awake_bytes = awake.as_bytes(0).unwrap_or_else(|| panic!("frame {scale}"));
        assert_eq!(rest_bytes, awake_bytes, "{scale}");
        assert!(
            rest_bytes
                .chunks(4)
                .any(|pixel| pixel[3] == 255 && (pixel[0] != pixel[1] || pixel[1] != pixel[2])),
            "{scale}"
        );
        let letters_rest = fitted_image(
            "letters:AT",
            SLATE.soft,
            Tone::Rest,
            56.0,
            35.0,
            scale,
            &context,
        )
        .unwrap_or_else(|| panic!("letters rest {scale}"));
        let letters_awake = fitted_image(
            "letters:AT",
            SLATE.soft,
            Tone::Awake,
            56.0,
            35.0,
            scale,
            &context,
        )
        .unwrap_or_else(|| panic!("letters awake {scale}"));
        assert_ne!(
            letters_rest
                .as_bytes(0)
                .unwrap_or_else(|| panic!("frame {scale}")),
            letters_awake
                .as_bytes(0)
                .unwrap_or_else(|| panic!("frame {scale}")),
            "{scale}"
        );
    }
}

#[test]
fn the_tick_draws_ink() {
    let rendered = tick(0xf3f2f0, 32, 24).unwrap_or_else(|| panic!("tick"));
    let bytes = rendered.as_bytes(0).unwrap_or_else(|| panic!("frame tick"));
    assert!(bytes.chunks(4).any(|pixel| pixel[3] != 0));
}

#[test]
fn letters_follow_the_picker_rule() {
    assert_eq!(letters_for(PICKER_LABELS), PICKER_LETTERS);
    assert_eq!(letters_for(&["default", "work laptop"]), ["D", "WL"]);
}

#[test]
fn the_chevron_draws_ink() {
    let rendered = chevron(0xf3f2f0, 16, 28).unwrap_or_else(|| panic!("chevron"));
    let bytes = rendered
        .as_bytes(0)
        .unwrap_or_else(|| panic!("frame chevron"));
    assert!(bytes.chunks(4).any(|pixel| pixel[3] != 0));
}

fn sample_spec(name: &str) -> String {
    match name {
        "icon-corner" => "icon-corner:right".to_owned(),
        "aa" => "aa:22".to_owned(),
        "desktop-theme" => "desktop-theme:slate".to_owned(),
        "web-theme" => "web-theme:slate".to_owned(),
        "swatch" => "swatch:violet".to_owned(),
        "letters" => "letters:AT".to_owned(),
        "preset" => "preset:88,92".to_owned(),
        "format" => "format:MP4".to_owned(),
        "terminal-session" => "terminal-session:claude".to_owned(),
        "local-engine" => "local-engine:onnx".to_owned(),
        "display-mode" => "display-mode:2560x1440".to_owned(),
        other => other.to_owned(),
    }
}
