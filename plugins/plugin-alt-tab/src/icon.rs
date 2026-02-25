use crate::platform::RgbaImage;
use gpui::RenderImage;
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) fn build_icon_cache(
    raw_icons: HashMap<String, RgbaImage>,
) -> HashMap<String, Arc<RenderImage>> {
    let mut cache: HashMap<String, Arc<RenderImage>> = HashMap::new();
    for (app_name, icon) in raw_icons {
        let buf = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(
            icon.width as u32,
            icon.height as u32,
            icon.data,
        );
        if let Some(buf) = buf {
            let frame = image::Frame::new(buf);
            cache.insert(
                app_name,
                Arc::new(RenderImage::new(smallvec::smallvec![frame])),
            );
        }
    }
    cache
}
