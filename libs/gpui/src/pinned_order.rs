#[derive(Clone, Debug, Default)]
pub struct PinnedOrder<T: Clone + Eq> {
    order: Vec<T>,
}

impl<T: Clone + Eq> PinnedOrder<T> {
    pub fn new() -> Self {
        Self { order: Vec::new() }
    }

    pub fn reset(&mut self) {
        self.order.clear();
    }

    pub fn apply(&mut self, current: &[T]) -> Vec<T> {
        let mut pinned: Vec<T> = self
            .order
            .iter()
            .filter(|id| current.contains(id))
            .cloned()
            .collect();
        for id in current {
            if !pinned.contains(id) {
                pinned.push(id.clone());
            }
        }
        self.order = pinned.clone();
        pinned
    }
}

#[cfg(test)]
mod tests {
    use super::PinnedOrder;

    #[test]
    fn the_first_apply_adopts_the_incoming_order() {
        let mut pin = PinnedOrder::new();
        assert_eq!(pin.apply(&[3, 1, 2]), vec![3, 1, 2]);
    }

    #[test]
    fn a_reshuffle_keeps_the_pinned_order() {
        let mut pin = PinnedOrder::new();
        pin.apply(&[3, 1, 2]);
        assert_eq!(pin.apply(&[1, 2, 3]), vec![3, 1, 2]);
    }

    #[test]
    fn newcomers_append_and_departures_drop() {
        let mut pin = PinnedOrder::new();
        pin.apply(&[3, 1, 2]);
        assert_eq!(pin.apply(&[4, 2, 3]), vec![3, 2, 4]);
    }

    #[test]
    fn reset_readopts_the_incoming_order() {
        let mut pin = PinnedOrder::new();
        pin.apply(&[3, 1, 2]);
        pin.reset();
        assert_eq!(pin.apply(&[1, 2, 3]), vec![1, 2, 3]);
    }
}
