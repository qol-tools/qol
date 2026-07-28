use std::collections::{HashSet, VecDeque};

const CAPACITY: usize = 4096;

#[derive(Default)]
pub struct ReplayWindow {
    order: VecDeque<[u8; 16]>,
    seen: HashSet<[u8; 16]>,
}

impl ReplayWindow {
    pub fn insert(&mut self, nonce: [u8; 16]) -> bool {
        if !self.seen.insert(nonce) {
            return false;
        }
        self.order.push_back(nonce);
        if self.order.len() <= CAPACITY {
            return true;
        }
        if let Some(expired) = self.order.pop_front() {
            self.seen.remove(&expired);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_is_rejected_until_it_leaves_the_window() {
        let mut window = ReplayWindow::default();
        let first = [1; 16];

        assert!(window.insert(first));
        assert!(!window.insert(first));
        for value in 0..=CAPACITY {
            let mut nonce = [0; 16];
            nonce[..8].copy_from_slice(&(value as u64).to_be_bytes());
            assert!(window.insert(nonce));
        }
        assert!(window.insert(first));
    }
}
