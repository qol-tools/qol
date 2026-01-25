use proptest::prelude::*;

mod common;
use common::config;

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

proptest! {
    #![proptest_config(config())]

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
