use proptest::prelude::*;
use std::collections::HashSet;

struct FilterDelegate {
    items: Vec<String>,
    matches: Vec<String>,
}

impl FilterDelegate {
    fn new(items: Vec<String>) -> Self {
        Self {
            matches: items.clone(),
            items,
        }
    }

    fn filter(&mut self, query: &str) {
        if query.is_empty() {
            self.matches = self.items.clone();
        } else {
            let q = query.to_lowercase();
            self.matches = self.items.iter().filter(|i| i.to_lowercase().contains(&q)).cloned().collect();
        }
    }
}

struct ListNavState {
    selected: usize,
    max_index: usize,
}

impl ListNavState {
    fn new(item_count: usize) -> Self {
        Self {
            selected: 0,
            max_index: item_count.saturating_sub(1),
        }
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self) {
        if self.selected < self.max_index {
            self.selected += 1;
        }
    }
}

#[derive(Clone)]
struct SectionEntry {
    is_header: bool,
}

struct SectionNavState {
    entries: Vec<SectionEntry>,
    selected: usize,
}

impl SectionNavState {
    fn new(entries: Vec<SectionEntry>) -> Self {
        let selected = entries.iter().position(|e| !e.is_header).unwrap_or(0);
        Self { entries, selected }
    }

    fn move_up(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let mut index = self.selected;
        while index > 0 {
            index -= 1;
            if !self.entries[index].is_header {
                self.selected = index;
                return;
            }
        }
    }

    fn move_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let mut index = self.selected;
        while index + 1 < self.entries.len() {
            index += 1;
            if !self.entries[index].is_header {
                self.selected = index;
                return;
            }
        }
    }
}

fn calculate_window_height(item_count: usize, header_height: f32, item_height: f32, max_visible: usize) -> f32 {
    let visible = item_count.min(max_visible);
    header_height + (visible as f32 * item_height)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn prop_filter_matches_always_contain_query(
        query in "[a-zA-Z0-9]{0,20}",
        items in prop::collection::vec("[a-zA-Z0-9]{1,30}", 0..50)
    ) {
        let mut delegate = FilterDelegate::new(items);
        delegate.filter(&query);

        let q = query.to_lowercase();
        for item in &delegate.matches {
            prop_assert!(
                item.to_lowercase().contains(&q),
                "Match '{}' does not contain query '{}'", item, query
            );
        }
    }

    #[test]
    fn prop_filter_results_subset_of_items(
        query in "[a-zA-Z0-9]{0,20}",
        items in prop::collection::vec("[a-zA-Z0-9]{1,30}", 0..50)
    ) {
        let mut delegate = FilterDelegate::new(items.clone());
        delegate.filter(&query);

        prop_assert!(
            delegate.matches.len() <= items.len(),
            "Filtered {} items but only had {} original", delegate.matches.len(), items.len()
        );

        for m in &delegate.matches {
            prop_assert!(
                items.contains(m),
                "Match '{}' not in original items", m
            );
        }
    }

    #[test]
    fn prop_filter_empty_query_returns_all(
        items in prop::collection::vec("[a-zA-Z0-9]{1,30}", 0..50)
    ) {
        let mut delegate = FilterDelegate::new(items.clone());
        delegate.filter("");

        prop_assert_eq!(
            delegate.matches.len(),
            items.len(),
            "Empty query should return all items"
        );
    }

    #[test]
    fn prop_filter_case_insensitive(
        base_query in "[a-z]{1,10}",
        items in prop::collection::vec("[a-zA-Z]{1,20}", 1..20)
    ) {
        let mut delegate_lower = FilterDelegate::new(items.clone());
        let mut delegate_upper = FilterDelegate::new(items);

        delegate_lower.filter(&base_query.to_lowercase());
        delegate_upper.filter(&base_query.to_uppercase());

        prop_assert_eq!(
            delegate_lower.matches.len(),
            delegate_upper.matches.len(),
            "Case sensitivity mismatch"
        );
    }

    #[test]
    fn prop_filter_case_insensitive_same_matches(
        base_query in "[a-z]{1,10}",
        items in prop::collection::vec("[a-zA-Z]{1,20}", 1..20)
    ) {
        let mut delegate_lower = FilterDelegate::new(items.clone());
        let mut delegate_upper = FilterDelegate::new(items);

        delegate_lower.filter(&base_query.to_lowercase());
        delegate_upper.filter(&base_query.to_uppercase());

        let lower: HashSet<_> = delegate_lower.matches.iter().cloned().collect();
        let upper: HashSet<_> = delegate_upper.matches.iter().cloned().collect();

        prop_assert_eq!(lower, upper, "Case insensitive matches differ");
    }

    #[test]
    fn prop_filter_preserves_order(
        query in "[a-zA-Z0-9]{0,5}",
        items in prop::collection::vec("[a-zA-Z0-9]{1,12}", 0..50)
    ) {
        let mut delegate = FilterDelegate::new(items.clone());
        delegate.filter(&query);

        let q = query.to_lowercase();
        let expected: Vec<String> = items
            .iter()
            .filter(|i| query.is_empty() || i.to_lowercase().contains(&q))
            .cloned()
            .collect();

        prop_assert_eq!(delegate.matches, expected, "Filtered order mismatch");
    }

    #[test]
    fn prop_filter_no_false_negatives(
        items in prop::collection::vec("[a-zA-Z]{3,15}", 1..30)
    ) {
        for item in &items {
            if item.len() >= 2 {
                let substring = &item[0..2];
                let mut delegate = FilterDelegate::new(items.clone());
                delegate.filter(substring);

                prop_assert!(
                    delegate.matches.contains(item),
                    "Item '{}' should match its own substring '{}'", item, substring
                );
            }
        }
    }

    #[test]
    fn prop_nav_selection_always_in_bounds(
        item_count in 1usize..100,
        moves in prop::collection::vec(prop::bool::ANY, 0..200)
    ) {
        let mut state = ListNavState::new(item_count);

        for go_down in moves {
            if go_down {
                state.move_down();
            } else {
                state.move_up();
            }

            prop_assert!(
                state.selected <= state.max_index,
                "Selection {} exceeded max {}", state.selected, state.max_index
            );
        }
    }

    #[test]
    fn prop_nav_down_then_up_returns_to_start(
        item_count in 2usize..50,
        steps in 1usize..20
    ) {
        let mut state = ListNavState::new(item_count);
        let steps = steps.min(item_count - 1);

        for _ in 0..steps {
            state.move_down();
        }
        for _ in 0..steps {
            state.move_up();
        }

        prop_assert_eq!(state.selected, 0, "Should return to start after equal up/down moves");
    }

    #[test]
    fn prop_nav_cannot_go_below_zero(
        item_count in 1usize..50,
        up_presses in 1usize..100
    ) {
        let mut state = ListNavState::new(item_count);

        for _ in 0..up_presses {
            state.move_up();
        }

        prop_assert_eq!(state.selected, 0, "Selection should stay at 0");
    }

    #[test]
    fn prop_nav_cannot_exceed_max(
        item_count in 1usize..50,
        down_presses in 1usize..100
    ) {
        let mut state = ListNavState::new(item_count);
        let max_index = item_count.saturating_sub(1);

        for _ in 0..down_presses {
            state.move_down();
        }

        prop_assert!(state.selected <= max_index, "Selection {} exceeded max {}", state.selected, max_index);
    }

    #[test]
    fn prop_nav_zero_items_stays_zero(
        moves in prop::collection::vec(prop::bool::ANY, 0..200)
    ) {
        let mut state = ListNavState::new(0);

        for go_down in moves {
            if go_down {
                state.move_down();
            } else {
                state.move_up();
            }

            prop_assert_eq!(state.selected, 0, "Selection should remain 0");
            prop_assert_eq!(state.max_index, 0, "Max index should remain 0");
        }
    }

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
    fn prop_section_nav_never_selects_header(
        headers in prop::collection::vec(prop::bool::ANY, 1..30),
        moves in prop::collection::vec(prop::bool::ANY, 0..200)
    ) {
        let entries: Vec<SectionEntry> = headers
            .into_iter()
            .map(|is_header| SectionEntry { is_header })
            .collect();
        let has_item = entries.iter().any(|e| !e.is_header);
        let mut state = SectionNavState::new(entries);

        if !has_item {
            return Ok(());
        }

        for go_down in moves {
            if go_down {
                state.move_down();
            } else {
                state.move_up();
            }
            prop_assert!(
                !state.entries[state.selected].is_header,
                "Selected index {} is a header", state.selected
            );
        }
    }

    #[test]
    fn prop_section_nav_stays_in_bounds(
        headers in prop::collection::vec(prop::bool::ANY, 1..30),
        moves in prop::collection::vec(prop::bool::ANY, 0..200)
    ) {
        let entries: Vec<SectionEntry> = headers
            .into_iter()
            .map(|is_header| SectionEntry { is_header })
            .collect();
        let mut state = SectionNavState::new(entries);

        if state.entries.is_empty() {
            return Ok(());
        }

        for go_down in moves {
            if go_down {
                state.move_down();
            } else {
                state.move_up();
            }
            prop_assert!(
                state.selected < state.entries.len(),
                "Selection {} out of bounds {}", state.selected, state.entries.len()
            );
        }
    }

    #[test]
    fn prop_section_nav_up_then_down_reversible(
        headers in prop::collection::vec(prop::bool::ANY, 1..30),
        steps in 0usize..20
    ) {
        let entries: Vec<SectionEntry> = headers
            .into_iter()
            .map(|is_header| SectionEntry { is_header })
            .collect();
        let has_item = entries.iter().any(|e| !e.is_header);
        let mut state = SectionNavState::new(entries);

        if !has_item {
            return Ok(());
        }

        let start = state.selected;
        for _ in 0..steps {
            state.move_down();
        }
        for _ in 0..steps {
            state.move_up();
        }

        prop_assert_eq!(state.selected, start, "Selection should return to start");
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

    #[test]
    fn prop_filter_then_nav_stays_in_filtered_bounds(
        query in "[a-z]{0,3}",
        items in prop::collection::vec("[a-zA-Z]{2,10}", 5..30),
        moves in prop::collection::vec(prop::bool::ANY, 0..50)
    ) {
        let mut delegate = FilterDelegate::new(items);
        delegate.filter(&query);

        if delegate.matches.is_empty() {
            return Ok(());
        }

        let mut state = ListNavState::new(delegate.matches.len());

        for go_down in moves {
            if go_down {
                state.move_down();
            } else {
                state.move_up();
            }

            prop_assert!(
                state.selected < delegate.matches.len(),
                "Selection {} out of bounds for {} filtered items",
                state.selected,
                delegate.matches.len()
            );
        }
    }
}

const VALID_ICON_EXTENSIONS: &[&str] = &["png", "svg", "jpg", "jpeg", "webp"];

#[derive(Debug, Clone, PartialEq)]
enum IconSource {
    Path(String),
    Placeholder,
    None,
}

struct IconResolver;

impl IconResolver {
    fn new() -> Self {
        Self
    }

    fn is_valid_extension(path: &str) -> bool {
        path.rsplit('.').next()
            .map(|ext| VALID_ICON_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
            .unwrap_or(false)
    }

    fn has_path_traversal(path: &str) -> bool {
        path.contains("..") || path.contains('\0')
    }

    fn resolve(&self, icon_path: &str) -> IconSource {
        if icon_path.is_empty() {
            return IconSource::None;
        }

        if Self::has_path_traversal(icon_path) {
            return IconSource::Placeholder;
        }

        if !Self::is_valid_extension(icon_path) {
            return IconSource::Placeholder;
        }

        IconSource::Path(icon_path.to_string())
    }
}

#[derive(Debug, Clone)]
struct IconSize {
    width: u32,
    height: u32,
}

impl IconSize {
    const MIN: u32 = 8;
    const MAX: u32 = 128;

    fn new(size: u32) -> Self {
        let clamped = size.clamp(Self::MIN, Self::MAX);
        Self { width: clamped, height: clamped }
    }

    fn from_dimensions(width: u32, height: u32) -> Self {
        Self {
            width: width.clamp(Self::MIN, Self::MAX),
            height: height.clamp(Self::MIN, Self::MAX),
        }
    }

    fn fits_in_row(&self, row_height: u32, padding: u32) -> bool {
        self.height + padding * 2 <= row_height
    }
}

struct ListItemWithIcon {
    label: String,
    icon: IconSource,
    icon_size: IconSize,
}

impl ListItemWithIcon {
    fn new(label: String, icon_path: &str, icon_size: u32, resolver: &IconResolver) -> Self {
        Self {
            label,
            icon: resolver.resolve(icon_path),
            icon_size: IconSize::new(icon_size),
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn prop_icon_valid_extensions_accepted(
        name in "[a-zA-Z0-9_-]{1,20}",
        ext in prop::sample::select(VALID_ICON_EXTENSIONS.to_vec())
    ) {
        let path = format!("{}.{}", name, ext);
        let resolver = IconResolver::new();
        let result = resolver.resolve(&path);

        prop_assert!(
            matches!(result, IconSource::Path(_)),
            "Valid extension '{}' should be accepted, got {:?}", ext, result
        );
    }

    #[test]
    fn prop_icon_valid_extensions_case_insensitive(
        name in "[a-zA-Z0-9_-]{1,20}",
        ext in prop::sample::select(VALID_ICON_EXTENSIONS.to_vec())
    ) {
        let ext_upper = ext.to_uppercase();
        let path = format!("{}.{}", name, ext_upper);
        let resolver = IconResolver::new();
        let result = resolver.resolve(&path);

        prop_assert!(
            matches!(result, IconSource::Path(_)),
            "Uppercase extension '{}' should be accepted, got {:?}", ext_upper, result
        );
    }

    #[test]
    fn prop_icon_invalid_extensions_use_placeholder(
        name in "[a-zA-Z0-9_-]{1,20}",
        ext in "(exe|dll|sh|bat|cmd|ps1|js|py)"
    ) {
        let path = format!("{}.{}", name, ext);
        let resolver = IconResolver::new();
        let result = resolver.resolve(&path);

        prop_assert_eq!(
            result,
            IconSource::Placeholder,
            "Invalid extension '{}' should use placeholder", ext
        );
    }

    #[test]
    fn prop_icon_path_traversal_blocked(
        prefix in "(|/tmp|/home/user)",
        traversal in "(\\.\\./|\\.\\.\\.)",
        suffix in "[a-zA-Z0-9]{1,10}\\.png"
    ) {
        let path = format!("{}{}{}", prefix, traversal, suffix);
        let resolver = IconResolver::new();
        let result = resolver.resolve(&path);

        prop_assert_eq!(
            result,
            IconSource::Placeholder,
            "Path traversal '{}' should be blocked", path
        );
    }

    #[test]
    fn prop_icon_null_byte_blocked(
        prefix in "[a-zA-Z0-9]{1,10}",
        suffix in "[a-zA-Z0-9]{1,10}\\.png"
    ) {
        let path = format!("{}\0{}", prefix, suffix);
        let resolver = IconResolver::new();
        let result = resolver.resolve(&path);

        prop_assert_eq!(
            result,
            IconSource::Placeholder,
            "Null byte in path should be blocked"
        );
    }

    #[test]
    fn prop_icon_empty_path_returns_none(
        _dummy in Just(())
    ) {
        let resolver = IconResolver::new();
        let result = resolver.resolve("");

        prop_assert_eq!(result, IconSource::None, "Empty path should return None");
    }

    #[test]
    fn prop_icon_size_clamped_to_bounds(size in 0u32..256) {
        let icon_size = IconSize::new(size);

        prop_assert!(
            icon_size.width >= IconSize::MIN && icon_size.width <= IconSize::MAX,
            "Width {} out of bounds [{}, {}]", icon_size.width, IconSize::MIN, IconSize::MAX
        );
        prop_assert!(
            icon_size.height >= IconSize::MIN && icon_size.height <= IconSize::MAX,
            "Height {} out of bounds [{}, {}]", icon_size.height, IconSize::MIN, IconSize::MAX
        );
    }

    #[test]
    fn prop_icon_size_from_dimensions_clamped(
        width in 0u32..256,
        height in 0u32..256
    ) {
        let icon_size = IconSize::from_dimensions(width, height);

        prop_assert!(
            icon_size.width >= IconSize::MIN && icon_size.width <= IconSize::MAX,
            "Width {} out of bounds [{}, {}]", icon_size.width, IconSize::MIN, IconSize::MAX
        );
        prop_assert!(
            icon_size.height >= IconSize::MIN && icon_size.height <= IconSize::MAX,
            "Height {} out of bounds [{}, {}]", icon_size.height, IconSize::MIN, IconSize::MAX
        );
    }

    #[test]
    fn prop_icon_fits_in_row_when_small_enough(
        icon_size in 8u32..64,
        row_height in 32u32..128,
        padding in 2u32..8
    ) {
        let icon = IconSize::new(icon_size);
        let fits = icon.fits_in_row(row_height, padding);
        let required_height = icon.height + padding * 2;

        prop_assert_eq!(
            fits,
            required_height <= row_height,
            "Icon {} + padding {} x 2 = {} vs row {}",
            icon.height, padding, required_height, row_height
        );
    }

    #[test]
    fn prop_list_item_icon_resolved_correctly(
        label in "[a-zA-Z ]{1,30}",
        icon_name in "[a-zA-Z0-9_-]{1,15}",
        icon_size in 16u32..48
    ) {
        let resolver = IconResolver::new();
        let path = format!("{}.png", icon_name);
        let item = ListItemWithIcon::new(label.clone(), &path, icon_size, &resolver);

        prop_assert!(!item.label.is_empty(), "Label should not be empty");
        prop_assert!(
            matches!(item.icon, IconSource::Path(_)),
            "Valid icon path should resolve to Path"
        );
        prop_assert!(
            item.icon_size.width >= IconSize::MIN && item.icon_size.width <= IconSize::MAX,
            "Icon size should be within bounds"
        );
    }
}
