use proptest::prelude::*;
use std::collections::HashSet;

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

proptest! {
    #![proptest_config(config())]

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
}
