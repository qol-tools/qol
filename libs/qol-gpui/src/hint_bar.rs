use qol_theme::{SPACE_GUTTER, SPACE_PAD, TEXT_KEYCAP, TEXT_MICRO};

const KEYCAP_PAD_X: f32 = 5.0;
const KEYCAP_BORDER: f32 = 1.0;
const KEY_LABEL_GAP: f32 = 6.0;
const CHIP_PAD_X: f32 = 6.0;
const LABEL_ADVANCE_PER_CHAR: f32 = 0.62;
const KEYCAP_ADVANCE_PER_CHAR: f32 = 0.62;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HintDescriptor {
    pub key: &'static str,
    pub label: &'static str,
    pub priority: u8,
    pub pinned: bool,
}

impl HintDescriptor {
    pub fn new(key: &'static str, label: &'static str, priority: u8) -> Self {
        Self {
            key,
            label,
            priority,
            pinned: false,
        }
    }

    pub fn pinned(key: &'static str, label: &'static str) -> Self {
        Self {
            key,
            label,
            priority: 0,
            pinned: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarItem {
    Hint(HintDescriptor),
    FixedWidth(f32),
    Spacer,
}

pub fn fit_hints(available_width: f32, items: &[BarItem]) -> Vec<BarItem> {
    let mut survivors = items.to_vec();
    while estimated_bar_width(&survivors) > available_width {
        let Some(index) = droppable_hint_index(&survivors) else {
            break;
        };
        survivors.remove(index);
    }
    survivors
}

pub fn estimated_bar_width(items: &[BarItem]) -> f32 {
    if items.is_empty() {
        return 0.0;
    }
    let content: f32 = items.iter().map(estimated_item_width).sum();
    content + (items.len() - 1) as f32 * SPACE_GUTTER + 2.0 * SPACE_PAD
}

pub fn estimated_chip_width(label: &str) -> f32 {
    estimated_label_width(label) + 2.0 * CHIP_PAD_X
}

fn estimated_item_width(item: &BarItem) -> f32 {
    match item {
        BarItem::Hint(descriptor) => {
            estimated_keycap_width(descriptor.key)
                + KEY_LABEL_GAP
                + estimated_label_width(descriptor.label)
        }
        BarItem::FixedWidth(width) => *width,
        BarItem::Spacer => 0.0,
    }
}

fn estimated_keycap_width(text: &str) -> f32 {
    text.chars().count() as f32 * KEYCAP_ADVANCE_PER_CHAR * TEXT_KEYCAP
        + 2.0 * KEYCAP_PAD_X
        + 2.0 * KEYCAP_BORDER
}

fn estimated_label_width(text: &str) -> f32 {
    text.chars().count() as f32 * LABEL_ADVANCE_PER_CHAR * TEXT_MICRO
}

fn droppable_hint_index(items: &[BarItem]) -> Option<usize> {
    let mut lowest = u8::MAX;
    let mut index = None;
    for (position, item) in items.iter().enumerate() {
        if let BarItem::Hint(descriptor) = item {
            if !descriptor.pinned && descriptor.priority <= lowest {
                lowest = descriptor.priority;
                index = Some(position);
            }
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launcher_items() -> Vec<BarItem> {
        vec![
            BarItem::Hint(HintDescriptor::new("\u{23CE}", "open", 2)),
            BarItem::Hint(HintDescriptor::new("\u{2191}\u{2193}", "move", 2)),
            BarItem::Hint(HintDescriptor::new("\u{21E5}", "mode", 1)),
            BarItem::FixedWidth(estimated_chip_width("Apps")),
            BarItem::Hint(HintDescriptor::new("type", "search", 0)),
            BarItem::Spacer,
            BarItem::Hint(HintDescriptor::pinned("esc", "dismiss")),
        ]
    }

    fn hint_keys(items: &[BarItem]) -> Vec<&'static str> {
        items
            .iter()
            .filter_map(|item| match item {
                BarItem::Hint(descriptor) => Some(descriptor.key),
                _ => None,
            })
            .collect()
    }

    fn has_fixed_chip(items: &[BarItem]) -> bool {
        items
            .iter()
            .any(|item| matches!(item, BarItem::FixedWidth(_)))
    }

    fn has_spacer(items: &[BarItem]) -> bool {
        items.iter().any(|item| matches!(item, BarItem::Spacer))
    }

    #[test]
    fn a_generous_width_keeps_every_hint() {
        let items = launcher_items();
        let width = estimated_bar_width(&items);
        assert_eq!(fit_hints(width, &items), items);
        assert_eq!(fit_hints(width + 1000.0, &items), items);
    }

    #[test]
    fn the_launcher_hint_set_fits_its_500_px_window() {
        let fit = fit_hints(500.0, &launcher_items());
        assert!(estimated_bar_width(&fit) <= 500.0);
        assert_eq!(
            hint_keys(&fit),
            ["\u{23CE}", "\u{2191}\u{2193}", "\u{21E5}", "esc"]
        );
        assert!(has_fixed_chip(&fit));
        assert!(has_spacer(&fit));
    }

    #[test]
    fn shrinking_width_drops_hints_in_priority_order() {
        let items = launcher_items();
        for (width, expected_keys) in [
            (
                600.0,
                vec!["\u{23CE}", "\u{2191}\u{2193}", "\u{21E5}", "type", "esc"],
            ),
            (
                500.0,
                vec!["\u{23CE}", "\u{2191}\u{2193}", "\u{21E5}", "esc"],
            ),
            (400.0, vec!["\u{23CE}", "\u{2191}\u{2193}", "esc"]),
            (300.0, vec!["\u{23CE}", "esc"]),
            (250.0, vec!["esc"]),
            (100.0, vec!["esc"]),
        ] {
            let fit = fit_hints(width, &items);
            assert_eq!(hint_keys(&fit), expected_keys, "width {width}");
            assert!(has_fixed_chip(&fit), "width {width}");
            assert!(has_spacer(&fit), "width {width}");
        }
    }

    #[test]
    fn equal_priority_drops_the_later_hint_first() {
        let items = vec![
            BarItem::Hint(HintDescriptor::new("a", "x", 1)),
            BarItem::Hint(HintDescriptor::new("b", "y", 1)),
        ];
        let fit = fit_hints(100.0, &items);
        assert_eq!(hint_keys(&fit), ["a"]);
    }

    #[test]
    fn the_pinned_hint_is_never_dropped() {
        let pinned_only = vec![BarItem::Hint(HintDescriptor::pinned("esc", "dismiss"))];
        assert_eq!(fit_hints(0.0, &pinned_only), pinned_only);
        let fit = fit_hints(1.0, &launcher_items());
        assert_eq!(hint_keys(&fit), ["esc"]);
        assert!(has_fixed_chip(&fit));
        assert!(has_spacer(&fit));
    }

    #[test]
    fn surviving_bar_width_never_exceeds_the_available_width() {
        let items = launcher_items();
        let floor = estimated_bar_width(&[
            BarItem::FixedWidth(estimated_chip_width("Apps")),
            BarItem::Spacer,
            BarItem::Hint(HintDescriptor::pinned("esc", "dismiss")),
        ]);
        for width in [
            floor,
            floor + 1.0,
            250.0,
            300.0,
            400.0,
            500.0,
            600.0,
            1000.0,
        ] {
            let fit = fit_hints(width, &items);
            assert!(
                estimated_bar_width(&fit) <= width,
                "width {width} is exceeded"
            );
        }
    }

    #[test]
    fn an_empty_bar_fits_any_width() {
        let fit = fit_hints(0.0, &[]);
        assert!(fit.is_empty());
        assert_eq!(estimated_bar_width(&fit), 0.0);
    }
}
