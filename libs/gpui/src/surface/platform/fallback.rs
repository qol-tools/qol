use gpui::{Pixels, Size};

use super::{dimensions_match, SurfacePlatform};

pub(crate) struct Platform;

impl SurfacePlatform for Platform {
    fn supports_native_reveal_gate() -> bool {
        false
    }

    fn required_layout_epoch(current: u64) -> u64 {
        current
    }

    fn viewport_matches(actual: Size<Pixels>, expected: Size<Pixels>, tolerance: f64) -> bool {
        dimensions_match(actual, expected, tolerance)
    }

    fn layout_confirmed(
        _current: u64,
        _required: u64,
        _observed: Size<Pixels>,
        _expected: Size<Pixels>,
        _tolerance: f64,
    ) -> bool {
        true
    }

    fn reveal_fail_open() -> bool {
        false
    }
}
