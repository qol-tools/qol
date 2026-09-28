mod bytes;
mod disc;
mod document;
mod font;
mod media;

use std::path::Path;

const PICTURES: [&str; 19] = [
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "avif", "heic", "heif", "ico",
    "jxl", "qoi", "tga", "psd", "exr", "hdr", "dds",
];

pub fn facts(path: &Path) -> Vec<String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let extension = extension.as_str();
    if PICTURES.contains(&extension) {
        return imagesize::size(path)
            .ok()
            .map(|size| format!("{} \u{d7} {}", size.width, size.height))
            .into_iter()
            .collect();
    }
    let single = match extension {
        "pdf" => document::pdf_pages(path).map(|pages| counted(pages, "page")),
        "odt" | "docx" => {
            document::office_pages(path, extension).map(|pages| counted(pages, "page"))
        }
        "odp" | "pptx" => {
            document::office_slides(path, extension).map(|slides| counted(slides, "slide"))
        }
        "ttf" | "otf" | "ttc" => {
            font::glyphs(path).map(|glyphs| format!("{} glyphs", grouped(glyphs)))
        }
        "iso" => disc::bootable(path).then(|| "bootable".to_owned()),
        _ => {
            return media::read(path, extension)
                .map(|media| media.facts())
                .unwrap_or_default()
        }
    };
    single.into_iter().collect()
}

fn counted(count: u64, word: &str) -> String {
    if count == 1 {
        format!("1 {word}")
    } else {
        format!("{} {word}s", grouped(count))
    }
}

fn grouped(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn duration(seconds: f64) -> String {
    let total = (seconds.round() as u64).max(1);
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn frame_rate(fps: f64) -> String {
    if (fps - fps.round()).abs() < 0.01 {
        format!("{} fps", fps.round())
    } else {
        let text = format!("{fps:.2}");
        format!("{} fps", text.trim_end_matches('0'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_read_like_the_board() {
        assert_eq!(counted(1, "page"), "1 page");
        assert_eq!(counted(148, "page"), "148 pages");
        assert_eq!(grouped(1_294), "1,294");
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(grouped(999), "999");
        assert_eq!(duration(252.4), "4:12");
        assert_eq!(duration(6128.0), "1:42:08");
        assert_eq!(duration(5.0), "0:05");
        assert_eq!(duration(0.4), "0:01");
        assert_eq!(frame_rate(59.97), "59.97 fps");
        assert_eq!(frame_rate(60.01), "60 fps");
        assert_eq!(frame_rate(23.976), "23.98 fps");
        assert_eq!(frame_rate(29.97), "29.97 fps");
        assert_eq!(frame_rate(25.5), "25.5 fps");
    }

    #[test]
    fn pictures_name_their_size() {
        let dir = tempfile::TempDir::new().unwrap();
        let png = dir.path().join("shot.PNG");
        let mut data = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        data.extend_from_slice(&5120u32.to_be_bytes());
        data.extend_from_slice(&1440u32.to_be_bytes());
        data.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        std::fs::write(&png, data).unwrap();
        assert_eq!(facts(&png), ["5120 \u{d7} 1440"]);
        assert!(facts(&dir.path().join("missing.png")).is_empty());
        assert!(facts(&dir.path().join("notes.txt")).is_empty());
    }
}
