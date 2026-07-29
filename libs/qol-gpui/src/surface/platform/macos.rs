use gpui::{px, size, Pixels, Size};

use super::{dimensions_match, SurfacePlatform};

pub(crate) struct Platform;

impl SurfacePlatform for Platform {
    fn supports_native_reveal_gate() -> bool {
        true
    }

    fn required_layout_epoch(current: u64) -> u64 {
        current
    }

    fn viewport_matches(actual: Size<Pixels>, expected: Size<Pixels>, tolerance: f64) -> bool {
        let native_expected = size(
            px(expected.width.to_f64().ceil() as f32),
            px(expected.height.to_f64().ceil() as f32),
        );
        dimensions_match(actual, native_expected, tolerance)
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
}

#[cfg(test)]
mod tests {
    use gpui::{px, size};

    use super::{Platform, SurfacePlatform};

    #[test]
    fn viewport_matching_accounts_for_native_fractional_rounding() {
        let expected = size(px(1036.0), px(435.4569));

        assert!(Platform::viewport_matches(
            size(px(1036.0), px(437.0)),
            expected,
            1.0
        ));
        assert!(!Platform::viewport_matches(
            size(px(1036.0), px(438.0)),
            expected,
            1.0
        ));
    }
}
