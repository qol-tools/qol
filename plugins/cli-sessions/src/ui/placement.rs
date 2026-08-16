pub use qol_gpui::placement::Corner;

pub const CORNER_MARGIN: f32 = 16.0;

pub fn parse_corner(value: &str) -> Corner {
    match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "top-left" => Corner::TopLeft,
        "bottom-left" => Corner::BottomLeft,
        "bottom-right" => Corner::BottomRight,
        _ => Corner::TopRight,
    }
}
