use gpui::{Pixels, Size};

use super::{dimensions_match, SurfacePlatform};

pub(crate) struct Platform;

impl SurfacePlatform for Platform {
    fn supports_native_reveal_gate() -> bool {
        true
    }

    fn required_layout_epoch(current: u64) -> u64 {
        current.wrapping_add(1)
    }

    fn viewport_matches(actual: Size<Pixels>, expected: Size<Pixels>, tolerance: f64) -> bool {
        dimensions_match(actual, expected, tolerance)
    }

    fn layout_confirmed(
        current: u64,
        required: u64,
        observed: Size<Pixels>,
        expected: Size<Pixels>,
        tolerance: f64,
    ) -> bool {
        current >= required && dimensions_match(observed, expected, tolerance)
    }

    fn reveal_fail_open() -> bool {
        false
    }
}
