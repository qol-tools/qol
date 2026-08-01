use plugin_cli_sessions::selection::Selection;
use proptest::prelude::*;

/// (order, window to select, expected highlight index, expected resolved window)
type EdgeCase = (&'static [u64], Option<u64>, Option<usize>, Option<u64>);

// --- exact-output scenario tests ------------------------------------------

/// The production bug, end to end: a click/selection must keep following the
/// session it was put on, even as attention changes reorder the rows under it.
/// window ids and the NeedsYou-jumps-to-top move mirror the captured trace
/// (Brunata wid=3 sliding down as GitHub_Agent wid=5 flips to NeedsYou).
#[test]
fn selection_follows_the_session_through_attention_reorders() {
    let mut sel = Selection::default();

    // all YourTurn -> stable-sorted by window id
    let order1 = [2u64, 3, 5, 8];
    // user clicks Brunata (wid=3)
    sel.select(3);
    assert_eq!(sel.highlight_index(&order1), Some(1));
    assert_eq!(sel.resolved(&order1), Some(3));

    // wid=5 flips to NeedsYou and jumps to the top; Brunata is now row 2
    let order2 = [5u64, 2, 3, 8];
    assert_eq!(
        sel.resolved(&order2),
        Some(3),
        "Enter/click must still hit Brunata, not whoever took the top slot"
    );
    assert_eq!(sel.highlight_index(&order2), Some(2));

    // arrow-down steps within the CURRENT order, re-anchoring to wid=8
    sel.move_down(&order2);
    assert_eq!(sel.resolved(&order2), Some(8));

    // Brunata finishes and drops to the bottom; selection (wid=8) holds
    let order3 = [5u64, 2, 8, 3];
    assert_eq!(sel.resolved(&order3), Some(8));
    assert_eq!(sel.highlight_index(&order3), Some(2));

    // the anchored window closes -> selection falls back to the top row
    let order4 = [5u64, 2, 3];
    assert_eq!(
        sel.resolved(&order4),
        Some(5),
        "a vanished anchor falls back to the top, never to a stale id"
    );
}

#[test]
fn highlight_and_resolved_handle_edge_orders() {
    // (order, anchor to select, expected highlight index, expected resolved)
    let cases: [EdgeCase; 5] = [
        (&[], None, None, None),
        (&[], Some(7), None, None),
        (&[7], None, Some(0), Some(7)),
        (&[7, 8, 9], Some(8), Some(1), Some(8)),
        (&[7, 8, 9], Some(404), Some(0), Some(7)),
    ];
    for (order, anchor, expect_index, expect_resolved) in cases {
        let mut sel = Selection::default();
        if let Some(w) = anchor {
            sel.select(w);
        }
        assert_eq!(
            sel.highlight_index(order),
            expect_index,
            "order={order:?} anchor={anchor:?}"
        );
        assert_eq!(
            sel.resolved(order),
            expect_resolved,
            "order={order:?} anchor={anchor:?}"
        );
    }
}

#[test]
fn moves_clamp_at_both_ends() {
    let order = [10u64, 20, 30];
    let mut sel = Selection::default();
    sel.select(10);
    sel.move_up(&order); // already at top
    assert_eq!(sel.resolved(&order), Some(10));
    sel.move_down(&order);
    sel.move_down(&order);
    sel.move_down(&order); // past the end
    assert_eq!(sel.resolved(&order), Some(30));
}

// --- property invariants ---------------------------------------------------

fn unique_order(len: std::ops::Range<usize>) -> impl Strategy<Value = Vec<u64>> {
    prop::collection::hash_set(any::<u64>(), len).prop_map(|set| set.into_iter().collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// highlight_index is None exactly when the list is empty, else in bounds.
    #[test]
    fn prop_highlight_index_in_bounds(order in unique_order(0..16), anchor in any::<Option<u64>>()) {
        let mut sel = Selection::default();
        if let Some(w) = anchor { sel.select(w); }
        match sel.highlight_index(&order) {
            None => prop_assert!(order.is_empty()),
            Some(i) => {
                prop_assert!(!order.is_empty());
                prop_assert!(i < order.len());
            }
        }
    }

    /// resolved always names a window that is actually on screen (or None when empty).
    #[test]
    fn prop_resolved_is_on_screen(order in unique_order(0..16), anchor in any::<Option<u64>>()) {
        let mut sel = Selection::default();
        if let Some(w) = anchor { sel.select(w); }
        match sel.resolved(&order) {
            None => prop_assert!(order.is_empty()),
            Some(w) => prop_assert!(order.contains(&w)),
        }
    }

    /// Selecting a visible window makes every action resolve to exactly it.
    #[test]
    fn prop_select_anchors_resolution(order in unique_order(1..16), pick in any::<prop::sample::Index>()) {
        let w = order[pick.index(order.len())];
        let mut sel = Selection::default();
        sel.select(w);
        prop_assert_eq!(sel.resolved(&order), Some(w));
        prop_assert_eq!(sel.highlight_index(&order), order.iter().position(|&x| x == w));
    }

    /// THE regression guard: once a session is selected, reordering the rows
    /// in any way never changes which session the selection resolves to.
    #[test]
    fn prop_reorder_never_changes_selection(
        (order, shuffled, pick) in unique_order(1..16).prop_flat_map(|order| {
            let len = order.len();
            (Just(order.clone()), Just(order).prop_shuffle(), 0..len)
        })
    ) {
        let w = order[pick];
        let mut sel = Selection::default();
        sel.select(w);
        prop_assert_eq!(sel.resolved(&shuffled), Some(w));
        prop_assert_eq!(sel.highlight_index(&shuffled), shuffled.iter().position(|&x| x == w));
    }

    /// Any sequence of arrow moves keeps the selection on a visible row.
    #[test]
    fn prop_moves_stay_in_bounds(
        order in unique_order(1..16),
        start in any::<prop::sample::Index>(),
        moves in prop::collection::vec(any::<bool>(), 0..64),
    ) {
        let mut sel = Selection::default();
        sel.select(order[start.index(order.len())]);
        for down in moves {
            if down { sel.move_down(&order) } else { sel.move_up(&order) }
            let i = sel.highlight_index(&order).expect("non-empty order has a highlight");
            prop_assert!(i < order.len());
            prop_assert_eq!(sel.resolved(&order), Some(order[i]));
        }
    }
}
