use std::collections::HashSet;

pub struct FirstFillGate<T> {
    pending: Option<Vec<(u32, T)>>,
    failed: HashSet<u32>,
}

impl<T> FirstFillGate<T> {
    pub fn new(first_fill: bool) -> Self {
        Self {
            pending: first_fill.then(Vec::new),
            failed: HashSet::new(),
        }
    }

    pub fn note_failure(&mut self, wid: u32) {
        self.failed.insert(wid);
    }

    pub fn admit(&mut self, frames: Vec<(u32, T)>, visible: &[u32]) -> Option<Vec<(u32, T)>> {
        let Some(pending) = self.pending.as_mut() else {
            return Some(frames);
        };
        pending.extend(frames);
        let covered =
            |wid: &u32| self.failed.contains(wid) || pending.iter().any(|(seen, _)| seen == wid);
        if pending.is_empty() || !visible.iter().all(covered) {
            return None;
        }
        self.pending.take()
    }

    pub fn take_pending(&mut self) -> Option<Vec<(u32, T)>> {
        self.pending.take().filter(|pending| !pending.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::FirstFillGate;

    #[test]
    fn pass_through_when_frames_already_retained() {
        let mut gate: FirstFillGate<u8> = FirstFillGate::new(false);
        let out = gate.admit(vec![(1, 10)], &[1, 2, 3]);
        assert_eq!(out, Some(vec![(1, 10)]), "retained state must not gate");
    }

    #[test]
    fn holds_until_every_visible_window_has_a_frame() {
        let mut gate: FirstFillGate<u8> = FirstFillGate::new(true);
        assert_eq!(gate.admit(vec![(1, 10)], &[1, 2]), None, "partial fill");
        let out = gate.admit(vec![(2, 20)], &[1, 2]);
        assert_eq!(
            out,
            Some(vec![(1, 10), (2, 20)]),
            "complete fill must flush all held frames at once"
        );
        assert_eq!(
            gate.admit(vec![(1, 11)], &[1, 2]),
            Some(vec![(1, 11)]),
            "after flush the gate passes frames through"
        );
    }

    #[test]
    fn failed_windows_do_not_block_the_flush() {
        let mut gate: FirstFillGate<u8> = FirstFillGate::new(true);
        gate.note_failure(2);
        let out = gate.admit(vec![(1, 10)], &[1, 2]);
        assert_eq!(out, Some(vec![(1, 10)]), "failed wid counts as covered");
    }

    #[test]
    fn empty_pending_never_flushes() {
        let mut gate: FirstFillGate<u8> = FirstFillGate::new(true);
        assert_eq!(gate.admit(vec![], &[]), None);
        assert_eq!(gate.admit(vec![(1, 10)], &[1]), Some(vec![(1, 10)]));
    }

    #[test]
    fn take_pending_returns_partial_fill_for_retention() {
        let mut gate: FirstFillGate<u8> = FirstFillGate::new(true);
        assert_eq!(gate.admit(vec![(1, 10)], &[1, 2]), None);
        assert_eq!(gate.take_pending(), Some(vec![(1, 10)]));
        assert_eq!(gate.take_pending(), None, "drained gate holds nothing");
    }
}
