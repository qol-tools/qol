use plugin_cli_sessions::host::kitty_session_id;
use plugin_cli_sessions::selection::Selection;
use proptest::prelude::*;
use qol_terminal_sessions::SessionId;

type EdgeCase = (&'static [u64], Option<u64>, Option<usize>, Option<u64>);

fn ids(values: &[u64]) -> Vec<SessionId> {
    values.iter().copied().map(kitty_session_id).collect()
}

fn native(id: Option<SessionId>) -> Option<u64> {
    id.and_then(|id| id.native().parse().ok())
}

#[test]
fn selection_follows_the_session_through_attention_reorders() {
    let mut selection = Selection::default();
    let order1 = ids(&[2, 3, 5, 8]);
    selection.select(kitty_session_id(3));
    assert_eq!(selection.highlight_index(&order1), Some(1));
    assert_eq!(native(selection.resolved(&order1)), Some(3));

    let order2 = ids(&[5, 2, 3, 8]);
    assert_eq!(native(selection.resolved(&order2)), Some(3));
    assert_eq!(selection.highlight_index(&order2), Some(2));

    selection.move_down(&order2);
    assert_eq!(native(selection.resolved(&order2)), Some(8));

    let order3 = ids(&[5, 2, 8, 3]);
    assert_eq!(native(selection.resolved(&order3)), Some(8));
    assert_eq!(selection.highlight_index(&order3), Some(2));

    let order4 = ids(&[5, 2, 3]);
    assert_eq!(native(selection.resolved(&order4)), Some(5));
}

#[test]
fn highlight_and_resolved_handle_edge_orders() {
    let cases: [EdgeCase; 5] = [
        (&[], None, None, None),
        (&[], Some(7), None, None),
        (&[7], None, Some(0), Some(7)),
        (&[7, 8, 9], Some(8), Some(1), Some(8)),
        (&[7, 8, 9], Some(404), Some(0), Some(7)),
    ];
    for (order, anchor, expected_index, expected_resolved) in cases {
        let order = ids(order);
        let mut selection = Selection::default();
        if let Some(anchor) = anchor {
            selection.select(kitty_session_id(anchor));
        }
        assert_eq!(selection.highlight_index(&order), expected_index);
        assert_eq!(native(selection.resolved(&order)), expected_resolved);
    }
}

#[test]
fn moves_clamp_at_both_ends() {
    let order = ids(&[10, 20, 30]);
    let mut selection = Selection::default();
    selection.select(kitty_session_id(10));
    selection.move_up(&order);
    assert_eq!(native(selection.resolved(&order)), Some(10));
    selection.move_down(&order);
    selection.move_down(&order);
    selection.move_down(&order);
    assert_eq!(native(selection.resolved(&order)), Some(30));
}

fn unique_order(len: std::ops::Range<usize>) -> impl Strategy<Value = Vec<u64>> {
    prop::collection::hash_set(any::<u64>(), len).prop_map(|set| set.into_iter().collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn prop_highlight_index_in_bounds(order in unique_order(0..16), anchor in any::<Option<u64>>()) {
        let order = ids(&order);
        let mut selection = Selection::default();
        if let Some(anchor) = anchor { selection.select(kitty_session_id(anchor)); }
        match selection.highlight_index(&order) {
            None => prop_assert!(order.is_empty()),
            Some(index) => {
                prop_assert!(!order.is_empty());
                prop_assert!(index < order.len());
            }
        }
    }

    #[test]
    fn prop_resolved_is_on_screen(order in unique_order(0..16), anchor in any::<Option<u64>>()) {
        let order = ids(&order);
        let mut selection = Selection::default();
        if let Some(anchor) = anchor { selection.select(kitty_session_id(anchor)); }
        match selection.resolved(&order) {
            None => prop_assert!(order.is_empty()),
            Some(id) => prop_assert!(order.contains(&id)),
        }
    }

    #[test]
    fn prop_select_anchors_resolution(order in unique_order(1..16), pick in any::<prop::sample::Index>()) {
        let order = ids(&order);
        let id = order[pick.index(order.len())].clone();
        let mut selection = Selection::default();
        selection.select(id.clone());
        prop_assert_eq!(selection.resolved(&order), Some(id.clone()));
        prop_assert_eq!(selection.highlight_index(&order), order.iter().position(|candidate| candidate == &id));
    }

    #[test]
    fn prop_reorder_never_changes_selection(
        (order, shuffled, pick) in unique_order(1..16).prop_flat_map(|order| {
            let len = order.len();
            (Just(order.clone()), Just(order).prop_shuffle(), 0..len)
        })
    ) {
        let order = ids(&order);
        let shuffled = ids(&shuffled);
        let id = order[pick].clone();
        let mut selection = Selection::default();
        selection.select(id.clone());
        prop_assert_eq!(selection.resolved(&shuffled), Some(id.clone()));
        prop_assert_eq!(selection.highlight_index(&shuffled), shuffled.iter().position(|candidate| candidate == &id));
    }

    #[test]
    fn prop_moves_stay_in_bounds(
        order in unique_order(1..16),
        start in any::<prop::sample::Index>(),
        moves in prop::collection::vec(any::<bool>(), 0..64),
    ) {
        let order = ids(&order);
        let mut selection = Selection::default();
        selection.select(order[start.index(order.len())].clone());
        for down in moves {
            if down { selection.move_down(&order) } else { selection.move_up(&order) }
            let index = selection.highlight_index(&order).expect("non-empty order has a highlight");
            prop_assert!(index < order.len());
            prop_assert_eq!(selection.resolved(&order), Some(order[index].clone()));
        }
    }
}
