use std::collections::{HashSet, VecDeque};

const CAPACITY: usize = 4096;

#[derive(Default)]
pub struct ReplayWindow {
    order: VecDeque<[u8; 32]>,
    seen: HashSet<[u8; 32]>,
}

impl ReplayWindow {
    pub fn insert(&mut self, device_id: &[u8; 16], nonce: &[u8; 16]) -> bool {
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(device_id);
        key[16..].copy_from_slice(nonce);
        if !self.seen.insert(key) {
            return false;
        }
        self.order.push_back(key);
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
        let device = [1; 16];
        let first = [1; 16];

        assert!(window.insert(&device, &first));
        assert!(!window.insert(&device, &first));
        for value in 0..=CAPACITY {
            let mut nonce = [0; 16];
            nonce[..8].copy_from_slice(&(value as u64).to_be_bytes());
            assert!(window.insert(&device, &nonce));
        }
        assert!(window.insert(&device, &first));
    }

    #[test]
    fn the_same_nonce_from_two_devices_does_not_collide() {
        let mut window = ReplayWindow::default();
        let nonce = [5; 16];

        assert!(window.insert(&[1; 16], &nonce));
        assert!(window.insert(&[2; 16], &nonce));
        assert!(!window.insert(&[1; 16], &nonce));
    }
}
