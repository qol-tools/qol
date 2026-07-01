use crate::{Monitor, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind {
    Screenshot,
    Recording,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Low,
    Balanced,
    High,
}

impl Quality {
    pub fn from_crf(crf: i32) -> Self {
        match crf {
            ..=18 => Quality::High,
            19..=28 => Quality::Balanced,
            _ => Quality::Low,
        }
    }

    fn bits_per_pixel_per_frame(self) -> f64 {
        match self {
            Quality::High => 0.20,
            Quality::Balanced => 0.10,
            Quality::Low => 0.05,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    Video {
        pixels: u64,
        fps: u32,
        audio: bool,
        quality: Quality,
    },
}

const AUDIO_BYTES_PER_SEC: u64 = 24_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimate {
    pub rate_bps: u64,
    pub fixed: u64,
}

pub fn estimate(capture: &Capture) -> Estimate {
    match capture {
        Capture::Video {
            pixels,
            fps,
            audio,
            quality,
        } => {
            let video = (*pixels as f64 * f64::from(*fps) * quality.bits_per_pixel_per_frame()
                / 8.0)
                .round();
            let audio = if *audio { AUDIO_BYTES_PER_SEC } else { 0 };
            Estimate {
                rate_bps: video as u64 + audio,
                fixed: 0,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Low,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Headroom {
    pub seconds: Option<u64>,
    pub level: Level,
}

const LOW_SECONDS: u64 = 300;
const CRITICAL_SECONDS: u64 = 60;
const LOW_BYTES: u64 = 2_000_000_000;
const CRITICAL_BYTES: u64 = 512_000_000;

pub fn headroom(estimate: &Estimate, available: u64) -> Headroom {
    if estimate.rate_bps == 0 {
        return Headroom {
            seconds: None,
            level: level_for(u64::MAX, available),
        };
    }

    let seconds = available.saturating_sub(estimate.fixed) / estimate.rate_bps;
    Headroom {
        seconds: Some(seconds),
        level: level_for(seconds, available),
    }
}

fn level_for(seconds: u64, available: u64) -> Level {
    if seconds < CRITICAL_SECONDS || available < CRITICAL_BYTES {
        Level::Critical
    } else if seconds < LOW_SECONDS || available < LOW_BYTES {
        Level::Low
    } else {
        Level::Ok
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayScale {
    pub bounds: Rect,
    pub scale: f64,
}

pub fn captured_pixels(selection: Rect, displays: &[DisplayScale]) -> u64 {
    displays
        .iter()
        .map(|display| {
            let area = intersection_area(selection, display.bounds) as f64;
            (area * display.scale * display.scale).round() as u64
        })
        .sum()
}

fn intersection_area(a: Rect, b: Rect) -> u64 {
    let bounds = Monitor {
        x: b.x,
        y: b.y,
        w: b.w,
        h: b.h,
    };
    crate::geometry::rect_intersection(a, bounds)
        .map(|r| r.w as u64 * r.h as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(pixels: u64, fps: u32, audio: bool, quality: Quality) -> Capture {
        Capture::Video {
            pixels,
            fps,
            audio,
            quality,
        }
    }

    #[test]
    fn quality_maps_from_crf_thresholds() {
        let cases = [
            (0, Quality::High),
            (18, Quality::High),
            (19, Quality::Balanced),
            (28, Quality::Balanced),
            (29, Quality::Low),
            (51, Quality::Low),
        ];
        for (crf, expected) in cases {
            assert_eq!(Quality::from_crf(crf), expected, "crf: {crf}");
        }
    }

    #[test]
    fn video_rate_scales_with_pixels_fps_quality_and_audio() {
        let cases = [
            (video(8000, 30, false, Quality::High), 6000),
            (video(8000, 30, false, Quality::Balanced), 3000),
            (video(8000, 30, false, Quality::Low), 1500),
            (video(8000, 30, true, Quality::High), 30000),
            (video(0, 60, false, Quality::High), 0),
        ];
        for (capture, expected_rate) in cases {
            let estimate = estimate(&capture);
            assert_eq!(estimate.rate_bps, expected_rate, "capture: {capture:?}");
            assert_eq!(estimate.fixed, 0, "video has no fixed cost: {capture:?}");
        }
    }

    #[test]
    fn headroom_classifies_seconds_and_absolute_floor() {
        let huge = 100_000_000_000;
        let cases = [
            (1000, 0, huge, Some(huge / 1000), Level::Ok),
            (1_000_000_000, 0, 30_000_000_000, Some(30), Level::Critical),
            (1_000_000_000, 0, 120_000_000_000, Some(120), Level::Low),
            (0, 0, huge, None, Level::Ok),
            (0, 0, 400_000_000, None, Level::Critical),
            (1000, 0, 1_500_000_000, Some(1_500_000), Level::Low),
        ];
        for (rate_bps, fixed, available, want_seconds, want_level) in cases {
            let got = headroom(&Estimate { rate_bps, fixed }, available);
            assert_eq!(
                got.seconds, want_seconds,
                "rate={rate_bps} avail={available}"
            );
            assert_eq!(got.level, want_level, "rate={rate_bps} avail={available}");
        }
    }

    #[test]
    fn captured_pixels_count_native_resolution_per_display() {
        let display_1x = DisplayScale {
            bounds: Rect {
                x: 0,
                y: 0,
                w: 1000,
                h: 1000,
            },
            scale: 1.0,
        };
        let display_2x = DisplayScale {
            bounds: Rect {
                x: 1000,
                y: 0,
                w: 1000,
                h: 1000,
            },
            scale: 2.0,
        };
        let displays = [display_1x, display_2x];

        // 100x100 fully inside the 1x display -> 10_000 px.
        let inside_1x = Rect {
            x: 10,
            y: 10,
            w: 100,
            h: 100,
        };
        assert_eq!(captured_pixels(inside_1x, &displays), 10_000);

        // 100x100 fully inside the 2x display -> 10_000 points * 4 = 40_000 px.
        let inside_2x = Rect {
            x: 1100,
            y: 10,
            w: 100,
            h: 100,
        };
        assert_eq!(captured_pixels(inside_2x, &displays), 40_000);

        // Straddle the seam: 100 pts on the 1x side + 100 pts on the 2x side, full height 1000.
        let straddle = Rect {
            x: 900,
            y: 0,
            w: 200,
            h: 1000,
        };
        let expected = 100 * 1000 + (100 * 1000) * 4;
        assert_eq!(captured_pixels(straddle, &displays), expected);
    }
}
