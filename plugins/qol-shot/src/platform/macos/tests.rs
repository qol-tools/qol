use std::path::{Path, PathBuf};

use crate::platform::{CaptureSegment, CaptureSession};
use crate::{Config, Monitor, Rect};

use super::{conversion, display, recording, selector, swift, system};

#[test]
fn non_mov_formats_capture_to_temporary_mov() {
    let output = Path::new("/tmp/recording.webm");
    assert_eq!(
        recording::native_capture_file_path(output),
        PathBuf::from("/tmp/recording.mov")
    );
}

#[test]
fn mov_format_captures_directly_to_output() {
    let output = Path::new("/tmp/recording.mov");
    assert_eq!(recording::native_capture_file_path(output), output);
}

#[test]
fn screencapture_recording_args_do_not_enable_click_spotlight() {
    let args = recording::screencapture_recording_args(
        Some(Rect {
            x: -1512,
            y: 458,
            w: 800,
            h: 600,
        }),
        Some(2),
        true,
    );

    assert_eq!(
        args,
        vec!["-v", "-D", "2", "-R", "-1512,458,800,600", "-x", "-g"]
    );
    assert!(!args.iter().any(|arg| arg == "-k"));
}

#[test]
fn screencapture_full_display_recording_args_avoid_area_selection() {
    let args = recording::screencapture_recording_args(None, Some(2), false);

    assert_eq!(args, vec!["-v", "-D", "2", "-x"]);
    assert!(!args.iter().any(|arg| arg == "-R"));
}

#[test]
fn rect_intersection_returns_overlap() {
    let rect = Rect {
        x: 1800,
        y: 100,
        w: 500,
        h: 400,
    };
    let monitor = Monitor {
        x: 1920,
        y: 0,
        w: 1920,
        h: 1080,
    };

    assert_eq!(
        display::rect_intersection(rect, monitor),
        Some(Rect {
            x: 1920,
            y: 100,
            w: 380,
            h: 400,
        })
    );
}

#[test]
fn selector_rect_mapper_clips_to_active_displays() {
    let rect = Rect {
        x: 1800,
        y: 100,
        w: 500,
        h: 400,
    };
    let displays = [
        Monitor {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        Monitor {
            x: 1920,
            y: 0,
            w: 1920,
            h: 1080,
        },
    ];

    assert_eq!(
        selector::map_selector_rect_to_capture(rect, &displays),
        Some(rect)
    );
}

#[test]
fn selector_rect_mapper_rejects_off_display_selection() {
    let rect = Rect {
        x: 4000,
        y: 100,
        w: 100,
        h: 100,
    };
    let displays = [Monitor {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    }];

    assert_eq!(
        selector::map_selector_rect_to_capture(rect, &displays),
        None
    );
}

#[test]
fn native_segment_composition_args_preserve_canvas_and_offsets() {
    let session = CaptureSession {
        output_file: Some(PathBuf::from("/tmp/final.mov")),
        capture_file: Some(PathBuf::from("/tmp/final.mov")),
        canvas: Some(Rect {
            x: 1800,
            y: 100,
            w: 500,
            h: 400,
        }),
        processes: Vec::new(),
        segments: vec![
            CaptureSegment {
                file: PathBuf::from("/tmp/left.mov"),
                rect: Rect {
                    x: 1800,
                    y: 100,
                    w: 120,
                    h: 400,
                },
                offset_x: 0,
                offset_y: 0,
            },
            CaptureSegment {
                file: PathBuf::from("/tmp/right.mov"),
                rect: Rect {
                    x: 1920,
                    y: 100,
                    w: 380,
                    h: 400,
                },
                offset_x: 120,
                offset_y: 0,
            },
        ],
    };

    assert_eq!(
        recording::native_segment_composition_args(
            &session,
            session.canvas.unwrap(),
            Path::new("/tmp/final.mov")
        ),
        vec![
            "500",
            "400",
            "/tmp/final.mov",
            "0",
            "0",
            "/tmp/left.mov",
            "120",
            "0",
            "/tmp/right.mov",
        ]
    );
}

#[test]
fn webm_conversion_uses_webm_codecs() {
    let args = conversion::conversion_args(
        Path::new("/tmp/native.mov"),
        Path::new("/tmp/out.webm"),
        &Config::default(),
    );
    assert!(args.iter().any(|arg| arg == "libvpx-vp9"));
    assert!(args.iter().any(|arg| arg == "libopus"));
    assert_eq!(args.last().map(String::as_str), Some("/tmp/out.webm"));
}

#[test]
fn ffmpeg_conversion_uses_configured_encoding_settings() {
    let cases = [
        ("/tmp/out.mp4", 24, "slow", "24", "slow"),
        ("/tmp/out.mkv", 0, "ultrafast", "0", "ultrafast"),
        ("/tmp/out.webm", 41, "ignored", "41", ""),
        ("/tmp/out.mp4", -2, "invalid", "0", "veryfast"),
        ("/tmp/out.webm", 87, "ignored", "51", ""),
    ];

    for (output, crf, preset, expected_crf, expected_preset) in cases {
        let config = config_with_encoding(crf, preset);
        let args =
            conversion::conversion_args(Path::new("/tmp/native.mov"), Path::new(output), &config);
        assert_arg_value(&args, "-crf", expected_crf);
        if !expected_preset.is_empty() {
            assert_arg_value(&args, "-preset", expected_preset);
        }
    }
}

#[test]
fn mp4_uses_avconvert_when_ffmpeg_is_missing() {
    let converter = conversion::converter_for(Path::new("/tmp/out.mp4"), false, true).unwrap();
    assert_eq!(converter, conversion::Converter::Avconvert);
}

#[test]
fn ffmpeg_is_preferred_when_available() {
    let converter = conversion::converter_for(Path::new("/tmp/out.mp4"), true, true).unwrap();
    assert_eq!(converter, conversion::Converter::Ffmpeg);
}

#[test]
fn webm_requires_ffmpeg() {
    assert!(conversion::converter_for(Path::new("/tmp/out.webm"), false, true).is_err());
}

#[test]
fn format_label_uses_uppercase_extension() {
    assert_eq!(
        system::output_format_label(Path::new("/tmp/out.mp4")),
        "MP4"
    );
}

#[test]
fn swift_helper_hash_includes_prelude_and_body() {
    assert_eq!(
        swift::swift_source_hash(swift::STATUS_OVERLAY_SWIFT),
        swift::swift_source_hash_with_prelude(swift::SWIFT_PRELUDE, swift::STATUS_OVERLAY_SWIFT),
        "helper hash should use the shared Swift prelude"
    );
    assert_ne!(
        swift::swift_source_hash(swift::STATUS_OVERLAY_SWIFT),
        swift::swift_source_hash(swift::CLIPBOARD_WRITER_SWIFT),
        "different helper bodies should use different cache keys"
    );
    assert_ne!(
        swift::swift_source_hash(swift::STATUS_OVERLAY_SWIFT),
        swift::swift_source_hash_with_prelude("changed prelude", swift::STATUS_OVERLAY_SWIFT),
        "prelude changes should invalidate cached helpers"
    );
}

fn config_with_encoding(crf: i32, preset: &str) -> Config {
    let mut config = Config::default();
    config.video.crf = crf;
    config.video.preset = preset.to_string();
    config
}

fn assert_arg_value(args: &[String], key: &str, expected: &str) {
    let Some(index) = args.iter().position(|arg| arg == key) else {
        panic!("missing arg {key} in {args:?}");
    };
    assert_eq!(args.get(index + 1).map(String::as_str), Some(expected));
}
