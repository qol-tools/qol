use std::path::Path;

use gpui::{ClipboardItem, Image, ImageFormat};

use crate::flow::FlowRow;

pub fn clipboard_item(row: &FlowRow) -> Option<ClipboardItem> {
    if row.spec.masked {
        return None;
    }
    if let Some(item) = row.spec.copy_image.as_deref().and_then(image_item) {
        return Some(item);
    }
    row.copy.clone().map(ClipboardItem::new_string)
}

fn image_item(path: &Path) -> Option<ClipboardItem> {
    let format = image_format(path)?;
    let bytes = std::fs::read(path).ok()?;
    Some(ClipboardItem::new_image(&Image::from_bytes(format, bytes)))
}

fn image_format(path: &Path) -> Option<ImageFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "webp" => ImageFormat::Webp,
        "gif" => ImageFormat::Gif,
        "svg" => ImageFormat::Svg,
        "bmp" => ImageFormat::Bmp,
        "tif" | "tiff" => ImageFormat::Tiff,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(copy: Option<&str>, copy_image: Option<&str>) -> FlowRow {
        FlowRow {
            title: "title".to_string(),
            subtitle: None,
            copy: copy.map(str::to_string),
            raw: serde_json::json!({}),
            spec: qol_plugin_api::launcher_flows::FlowRow {
                title: "title".to_string(),
                copy_image: copy_image.map(std::path::PathBuf::from),
                ..Default::default()
            },
        }
    }

    #[test]
    fn image_formats_follow_the_blob_extension() {
        assert_eq!(image_format(Path::new("/b/c.PNG")), Some(ImageFormat::Png));
        assert_eq!(
            image_format(Path::new("/b/c.jpeg")),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(
            image_format(Path::new("/b/c.webp")),
            Some(ImageFormat::Webp)
        );
        assert_eq!(image_format(Path::new("/b/c.txt")), None);
        assert_eq!(image_format(Path::new("/b/c")), None);
    }

    #[test]
    fn only_inline_text_or_a_readable_blob_is_copied_here() {
        let text = |row: &FlowRow| clipboard_item(row).and_then(|item| item.text());
        assert_eq!(text(&row(Some("copy"), None)).as_deref(), Some("copy"));
        assert!(clipboard_item(&row(None, None)).is_none());
        assert_eq!(
            text(&row(Some("copy"), Some("/nowhere/qol-missing.png"))).as_deref(),
            Some("copy")
        );
        let mut masked = row(Some("copy"), None);
        masked.spec.masked = true;
        assert!(clipboard_item(&masked).is_none());
    }

    #[test]
    fn a_readable_blob_is_copied_as_an_image() {
        let dir = std::env::temp_dir().join(format!("qol-launcher-copy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let blob = dir.join("clip.png");
        std::fs::write(&blob, [0x89, b'P', b'N', b'G']).unwrap();
        let item = clipboard_item(&row(Some("copy"), blob.to_str())).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(item.text(), None);
        assert_eq!(item.entries().len(), 1);
    }
}
