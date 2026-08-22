use gpui::{point, px, size, Bounds, Pixels};

use crate::ui::placement::Corner;

pub const STRIP_HEIGHT: f32 = 32.0;

pub fn strip_hover_bg(chrome_bg: u32, keycap_bg_rgba: u32) -> u32 {
    let tint = keycap_bg_rgba >> 8;
    let alpha = (keycap_bg_rgba & 0xff) as f32 / 255.0;
    qol_color::mix_rgb(chrome_bg, tint, alpha)
}

pub fn strip_bounds(expanded: Bounds<Pixels>, corner: Corner, strip_height: f32) -> Bounds<Pixels> {
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => expanded.origin.y,
        Corner::BottomLeft | Corner::BottomRight => {
            expanded.origin.y + expanded.size.height - px(strip_height)
        }
    };
    Bounds::new(
        point(expanded.origin.x, y),
        size(expanded.size.width, px(strip_height)),
    )
}

pub fn reanchor_expanded(
    expanded: Bounds<Pixels>,
    strip: Bounds<Pixels>,
    corner: Corner,
) -> Bounds<Pixels> {
    let x = match corner {
        Corner::TopLeft | Corner::BottomLeft => strip.origin.x,
        Corner::TopRight | Corner::BottomRight => {
            strip.origin.x + strip.size.width - expanded.size.width
        }
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => strip.origin.y,
        Corner::BottomLeft | Corner::BottomRight => {
            strip.origin.y + strip.size.height - expanded.size.height
        }
    };
    Bounds::new(point(x, y), expanded.size)
}

pub struct CollapseState {
    corner: Corner,
    collapsed: bool,
    expanded: Option<Bounds<Pixels>>,
}

impl CollapseState {
    pub fn new(corner: Corner) -> Self {
        Self {
            corner,
            collapsed: false,
            expanded: None,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    pub fn corner(&self) -> Corner {
        self.corner
    }

    pub fn collapse(&mut self, expanded: Bounds<Pixels>) -> Bounds<Pixels> {
        self.collapsed = true;
        self.expanded = Some(expanded);
        strip_bounds(expanded, self.corner, STRIP_HEIGHT)
    }

    pub fn expand(&mut self) -> Option<Bounds<Pixels>> {
        let expanded = self.expanded.take()?;
        self.collapsed = false;
        Some(expanded)
    }
}

#[cfg(test)]
mod tests {
    use super::strip_hover_bg;
    use qol_gpui::theme::{CliSessionsPalette, DARK_SYSTEM, LIGHT_SYSTEM};

    #[test]
    fn the_strip_hover_tint_stays_close_to_the_chrome_in_every_theme() {
        let channel = |color: u32, shift: u32| ((color >> shift) & 0xff) as i32;
        for (theme, system) in [("light", LIGHT_SYSTEM), ("dark", DARK_SYSTEM)] {
            let palette = CliSessionsPalette::from_system(system);
            let chrome = palette.chrome_bg;
            let hovered = strip_hover_bg(chrome, palette.keycap_bg_rgba);
            for shift in [16, 8, 0] {
                let drift = (channel(hovered, shift) - channel(chrome, shift)).abs();
                assert!(
                    drift <= 24,
                    "{theme} channel {shift}: hover {hovered:#08x} drifted {drift} from chrome \
                     {chrome:#08x}, nothing opaque sits behind the strip so the tint has to be \
                     blended into the chrome instead of painted over the empty window"
                );
            }
            assert_ne!(
                hovered, chrome,
                "{theme}: the strip must still react to the pointer"
            );
        }
    }
}
