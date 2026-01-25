use proptest::prelude::*;

mod common;
use common::config;

struct IndexingState {
    items: Vec<String>,
    indexed: usize,
    results: Vec<String>,
}

impl IndexingState {
    fn new(items: Vec<String>) -> Self {
        Self {
            items,
            indexed: 0,
            results: Vec::new(),
        }
    }

    fn index_more(&mut self, count: usize) {
        let new_indexed = self.indexed.saturating_add(count).min(self.items.len());
        self.indexed = new_indexed;
    }

    fn apply_query(&mut self, query: &str) {
        let q = query.to_lowercase();
        let indexed_slice = &self.items[..self.indexed];
        self.results = if query.is_empty() {
            indexed_slice.iter().cloned().collect()
        } else {
            indexed_slice
                .iter()
                .filter(|item| item.to_lowercase().contains(&q))
                .cloned()
                .collect()
        };
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn prop_indexing_results_are_subset_of_indexed_items(
        items in prop::collection::vec("[a-zA-Z0-9]{1,12}", 0..80),
        steps in prop::collection::vec((0usize..10, "[a-zA-Z0-9]{0,5}"), 1..50)
    ) {
        let mut state = IndexingState::new(items.clone());

        for (add_count, query) in steps {
            state.index_more(add_count);
            state.apply_query(&query);

            let indexed_slice = &state.items[..state.indexed];

            prop_assert!(
                state.results.len() <= indexed_slice.len(),
                "Results should be a subset of indexed items"
            );

            for item in &state.results {
                prop_assert!(
                    indexed_slice.contains(item),
                    "Result '{}' should be in indexed items", item
                );
                if !query.is_empty() {
                    prop_assert!(
                        item.to_lowercase().contains(&query.to_lowercase()),
                        "Result '{}' should match query '{}'", item, query
                    );
                }
            }
        }
    }

    #[test]
    fn prop_indexing_progress_is_monotonic(
        items in prop::collection::vec("[a-zA-Z0-9]{1,12}", 0..100),
        steps in prop::collection::vec(0usize..15, 1..50)
    ) {
        let mut state = IndexingState::new(items);
        let mut last = 0usize;

        for add_count in steps {
            state.index_more(add_count);
            prop_assert!(state.indexed >= last, "Indexed count should be non-decreasing");
            prop_assert!(state.indexed <= state.items.len(), "Indexed count cannot exceed total");
            last = state.indexed;
        }
    }
}
