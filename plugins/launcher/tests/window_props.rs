use proptest::prelude::*;

mod common;
use common::config;

fn calculate_window_height(
    item_count: usize,
    header_height: f32,
    item_height: f32,
    max_visible: usize,
) -> f32 {
    let visible = item_count.min(max_visible);
    header_height + (visible as f32 * item_height)
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn prop_window_height_grows_with_items(
        item_count in 0usize..100,
        header in 20.0f32..100.0,
        item_h in 20.0f32..50.0,
        max_visible in 5usize..20
    ) {
        let height = calculate_window_height(item_count, header, item_h, max_visible);
        let expected_visible = item_count.min(max_visible);
        let expected = header + (expected_visible as f32 * item_h);

        prop_assert!(
            (height - expected).abs() < 0.001,
            "Height {} != expected {}", height, expected
        );
    }

    #[test]
    fn prop_window_height_caps_at_max(
        item_count in 50usize..200,
        max_visible in 5usize..20
    ) {
        let header = 42.0;
        let item_h = 32.0;

        let height = calculate_window_height(item_count, header, item_h, max_visible);
        let max_height = header + (max_visible as f32 * item_h);

        prop_assert!(
            height <= max_height,
            "Height {} exceeded max {}", height, max_height
        );
    }

    #[test]
    fn prop_window_height_minimum_is_header(
        header in 20.0f32..100.0,
        item_h in 20.0f32..50.0,
        max_visible in 5usize..20
    ) {
        let height = calculate_window_height(0, header, item_h, max_visible);

        prop_assert!(
            (height - header).abs() < 0.001,
            "Empty list height {} should equal header {}", height, header
        );
    }

    #[test]
    fn prop_window_height_non_decreasing(
        count_a in 0usize..50,
        count_b in 0usize..50,
        header in 20.0f32..100.0,
        item_h in 20.0f32..50.0,
        max_visible in 5usize..20
    ) {
        let a = count_a.min(count_b);
        let b = count_a.max(count_b);
        let height_a = calculate_window_height(a, header, item_h, max_visible);
        let height_b = calculate_window_height(b, header, item_h, max_visible);

        prop_assert!(
            height_b + 0.001 >= height_a,
            "Height should not decrease: {} -> {}", height_a, height_b
        );
    }
}
