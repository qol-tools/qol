use std::path::{Path, PathBuf};

use crate::platform::{CaptureSegment, CaptureSession};
use crate::{Config, Monitor, Rect};

use super::{conversion, display, recording, selector, swift, system};

#[test]
fn capture_file_stages_every_format_in_the_work_directory() {
    let work = system::capture_work_dir();
    let cases = [
        "/Users/x/Videos/recording-2026.webm",
        "/Users/x/Videos/recording-2026.mp4",
        "/Users/x/Videos/recording-2026.mov",
    ];
    for output in cases {
        assert_eq!(
            recording::native_capture_file_path(Path::new(output)),
            work.join("recording-2026.mov"),
            "output: {output}"
        );
    }
}

#[test]
fn capture_work_dir_lives_under_the_system_temp_dir() {
    let work = system::capture_work_dir();
    assert_eq!(work.parent(), Some(std::env::temp_dir().as_path()));
    assert_eq!(
        work.file_name().and_then(|name| name.to_str()),
        Some("qol-shot")
    );
}

#[test]
fn finalization_moves_native_mov_and_reencodes_other_formats() {
    let cases = [
        (
            "/Users/x/Videos/clip.mov",
            recording::Finalization::MoveNative,
        ),
        (
            "/Users/x/Videos/clip.MOV",
            recording::Finalization::MoveNative,
        ),
        (
            "/Users/x/Videos/clip.mp4",
            recording::Finalization::Reencode,
        ),
        (
            "/Users/x/Videos/clip.webm",
            recording::Finalization::Reencode,
        ),
        (
            "/Users/x/Videos/clip.mkv",
            recording::Finalization::Reencode,
        ),
    ];
    for (output, expected) in cases {
        assert_eq!(
            recording::finalization_for(Path::new(output)),
            expected,
            "output: {output}"
        );
    }
}

#[test]
fn move_file_relocates_content_and_removes_the_source() {
    let dir = std::env::temp_dir().join(format!("qol-shot-move-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.mov");
    let destination = dir.join("nested/destination.mov");
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::write(&source, b"payload").unwrap();

    system::move_file(&source, &destination).unwrap();

    assert!(!source.exists(), "source should be gone");
    assert_eq!(std::fs::read(&destination).unwrap(), b"payload");
    std::fs::remove_dir_all(&dir).ok();
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
fn screencapture_frozen_args_capture_one_display_without_ui() {
    assert_eq!(
        system::screencapture_frozen_args(3),
        vec!["-D", "3", "-x", "-t", "png"]
    );
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
fn selector_rect_mapper_preserves_dragged_rect_touching_active_displays() {
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
fn selector_rect_mapper_preserves_canvas_across_vertically_offset_displays() {
    let rect = Rect {
        x: -1200,
        y: 0,
        w: 3200,
        h: 1400,
    };
    let displays = [
        Monitor {
            x: 0,
            y: 0,
            w: 2000,
            h: 1400,
        },
        Monitor {
            x: -1512,
            y: 458,
            w: 1512,
            h: 982,
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
fn native_segment_composition_args_carry_offsets_and_destination_sizes() {
    let session = CaptureSession {
        output_file: Some(PathBuf::from("/tmp/final.mov")),
        capture_file: Some(PathBuf::from("/tmp/final.mov")),
        canvas: Some(Rect {
            x: -1261,
            y: 685,
            w: 3298,
            h: 682,
        }),
        processes: Vec::new(),
        segments: vec![
            CaptureSegment {
                file: PathBuf::from("/tmp/main.mov"),
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 2560,
                    h: 1440,
                },
                offset_x: 1261,
                offset_y: -685,
            },
            CaptureSegment {
                file: PathBuf::from("/tmp/laptop.mov"),
                rect: Rect {
                    x: -1512,
                    y: 645,
                    w: 1512,
                    h: 982,
                },
                offset_x: -251,
                offset_y: -40,
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
            "3298",
            "682",
            "/tmp/final.mov",
            "1261",
            "-685",
            "2560",
            "1440",
            "/tmp/main.mov",
            "-251",
            "-40",
            "1512",
            "982",
            "/tmp/laptop.mov",
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
