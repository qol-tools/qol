mod audio;
mod matroska;
mod mp4;

use std::fs::File;
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Media {
    pub seconds: Option<f64>,
    pub size: Option<(u64, u64)>,
    pub fps: Option<f64>,
    pub bits: Option<u64>,
}

impl Media {
    pub fn facts(&self) -> Vec<String> {
        let mut facts = Vec::new();
        facts.extend(
            self.seconds
                .filter(|seconds| *seconds > 0.0)
                .map(super::duration),
        );
        facts.extend(
            self.size
                .map(|(width, height)| format!("{width} \u{d7} {height}")),
        );
        facts.extend(
            self.fps
                .filter(|fps| (1.0..=1000.0).contains(fps))
                .map(super::frame_rate),
        );
        if self.size.is_none() {
            facts.extend(
                self.bits
                    .filter(|bits| *bits > 0)
                    .map(|bits| format!("{bits} bit")),
            );
        }
        facts
    }
}

pub fn read(path: &Path, extension: &str) -> Option<Media> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    match extension {
        "mp4" | "m4v" | "mov" | "3gp" | "m4a" => mp4::read(&mut file, len),
        "mkv" | "webm" | "mka" => matroska::read(&mut file),
        "flac" => audio::flac(&mut file),
        "wav" => audio::wav(&mut file, len),
        "ogg" | "oga" | "opus" => audio::ogg(&mut file, len),
        "mp3" => audio::mp3(&mut file, len),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_shows_length_size_and_rate_and_audio_shows_depth() {
        let video = Media {
            seconds: Some(6128.0),
            size: Some((3840, 2160)),
            fps: Some(60.0),
            bits: Some(8),
        };
        assert_eq!(video.facts(), ["1:42:08", "3840 \u{d7} 2160", "60 fps"]);
        let audio = Media {
            seconds: Some(252.0),
            bits: Some(24),
            ..Media::default()
        };
        assert_eq!(audio.facts(), ["4:12", "24 bit"]);
        assert!(Media::default().facts().is_empty());
    }
}
