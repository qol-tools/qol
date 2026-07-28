use gpui::{Pixels, Size};

use super::SurfacePlatform;

pub(crate) struct Platform;

impl SurfacePlatform for Platform {
    fn supports_native_reveal_gate() -> bool {
        true
    }

    fn required_layout_epoch(current: u64) -> u64 {
        current.wrapping_add(1)
    }

    fn layout_confirmed(
        current: u64,
        required: u64,
        observed: Size<Pixels>,
        expected: Size<Pixels>,
        tolerance: f64,
    ) -> bool {
        current >= required
            && (observed.width.to_f64() - expected.width.to_f64()).abs() <= tolerance
            && (observed.height.to_f64() - expected.height.to_f64()).abs() <= tolerance
    }
}
