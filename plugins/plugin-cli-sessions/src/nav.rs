use crate::status::Status;

/// Index of the attention row to jump to next, over a priority-sorted slice
/// (NeedsYou before YourTurn, recent before old). With no cursor the jump lands
/// on the highest-priority row; with a cursor it advances one step and wraps, so
/// repeated presses walk the priority order.
pub fn next_attention(statuses: &[Status], current: Option<usize>) -> Option<usize> {
    let n = statuses.len();
    if n == 0 {
        return None;
    }
    let start = match current {
        Some(c) => (c + 1) % n,
        None => 0,
    };
    (0..n)
        .map(|offset| (start + offset) % n)
        .find(|&i| statuses[i].is_attention())
}
