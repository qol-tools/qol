use gpui::*;
use std::sync::Arc;

pub fn preview_tile(
    live_image: Option<&Arc<RenderImage>>,
    preview_path: &Option<String>,
    minimized_icon: Option<&Arc<RenderImage>>,
    width: f32,
    height: f32,
) -> AnyElement {
    if let Some(icon) = minimized_icon {
        return div()
            .w(px(width))
            .h(px(height))
            .bg(rgb(0x1e2130))
            .rounded_md()
            .border_1()
            .border_color(rgb(0x3a4252))
            .flex()
            .items_center()
            .justify_center()
            .child(
                img(icon.clone())
                    .w(px(48.0))
                    .h(px(48.0))
                    .rounded_md(),
            )
            .into_any_element();
    }
    if let Some(render_image) = live_image {
        img(render_image.clone())
            .w(px(width))
            .h(px(height))
            .object_fit(ObjectFit::Fill)
            .rounded_md()
            .into_any_element()
    } else if let Some(path) = preview_path {
        img(std::path::PathBuf::from(path))
            .w(px(width))
            .h(px(height))
            .object_fit(ObjectFit::Fill)
            .rounded_md()
            .into_any_element()
    } else {
        div()
            .w(px(width))
            .h(px(height))
            .bg(rgb(0x1e2130))
            .rounded_md()
            .border_1()
            .border_color(rgb(0x3a4252))
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(rgb(0x4a5268))
            .child("...")
            .into_any_element()
    }
}
