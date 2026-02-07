use proptest::prelude::*;

mod common;
use common::config;

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
    #![proptest_config(config())]

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
