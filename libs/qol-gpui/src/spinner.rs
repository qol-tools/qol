use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, Animation, AnimationExt as _, App, ElementId, Hsla, Pixels, RenderOnce, Window,
};

const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
const DEFAULT_SIZE: Pixels = px(14.);
const ROTATION_DURATION: Duration = Duration::from_millis(800);

#[derive(IntoElement)]
pub struct Spinner {
    id: ElementId,
    color: Hsla,
    size: Pixels,
}

impl Spinner {
    pub fn new(id: impl Into<ElementId>, color: impl Into<Hsla>) -> Self {
        Self {
            id: id.into(),
            color: color.into(),
            size: DEFAULT_SIZE,
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for Spinner {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(self.size)
            .h(self.size)
            .text_size(self.size)
            .text_color(self.color)
            .with_animation(
                self.id,
                Animation::new(ROTATION_DURATION).repeat(),
                |spinner, progress| spinner.child(FRAMES[frame_index(progress)]),
            )
    }
}

fn frame_index(progress: f32) -> usize {
    (progress * FRAMES.len() as f32) as usize % FRAMES.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_progress_maps_to_spinner_frames() {
        let cases = [
            (0., 0),
            (0.124, 0),
            (0.125, 1),
            (0.5, 4),
            (0.999, 7),
            (1., 0),
        ];

        for (progress, expected) in cases {
            assert_eq!(frame_index(progress), expected);
        }
    }
}
