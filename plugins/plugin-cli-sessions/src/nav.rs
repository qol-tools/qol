use crate::status::Status;

pub fn next_attention(statuses: &[Status], current: usize) -> Option<usize> {
    let n = statuses.len();
    if n == 0 {
        return None;
    }
    (1..=n)
        .map(|offset| (current + offset) % n)
        .find(|&i| statuses[i].is_attention())
}
