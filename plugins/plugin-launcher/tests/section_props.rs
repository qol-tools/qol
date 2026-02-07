use proptest::prelude::*;

mod common;
use common::config;

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

proptest! {
    #![proptest_config(config())]

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
}
