use std::collections::{BTreeSet, HashSet};

const CAPACITY: usize = 65_536;

#[derive(Default)]
pub(crate) struct ReplayWindow {
    expiry: BTreeSet<(u64, [u8; 32])>,
    seen: HashSet<[u8; 32]>,
}

impl ReplayWindow {
    pub(crate) fn insert(
        &mut self,
        device_id: &[u8; 16],
        nonce: &[u8; 16],
        expires_at_ms: u64,
        now_ms: u64,
    ) -> bool {
        self.expire(now_ms);
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(device_id);
        key[16..].copy_from_slice(nonce);
        if self.seen.contains(&key) || self.seen.len() >= CAPACITY {
            return false;
        }
        self.seen.insert(key);
        self.expiry.insert((expires_at_ms, key));
        true
    }

    fn expire(&mut self, now_ms: u64) {
        while let Some(&(expires_at_ms, key)) = self.expiry.first() {
            if expires_at_ms >= now_ms {
                return;
            }
            self.expiry.pop_first();
            self.seen.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nonce(value: u64) -> [u8; 16] {
        let mut nonce = [0; 16];
        nonce[..8].copy_from_slice(&value.to_be_bytes());
        nonce
    }

    #[test]
    fn a_nonce_is_refused_until_its_timestamp_expires() {
        let mut window = ReplayWindow::default();
        let device = [1; 16];

        assert!(window.insert(&device, &nonce(0), 1_000, 0));
        assert!(!window.insert(&device, &nonce(0), 1_000, 500));
        assert!(!window.insert(&device, &nonce(0), 1_000, 1_000));
        assert!(window.insert(&device, &nonce(0), 2_001, 1_001));
    }

    #[test]
    fn heavy_traffic_does_not_push_out_a_nonce_that_is_still_valid() {
        let mut window = ReplayWindow::default();
        let device = [1; 16];
        assert!(window.insert(&device, &nonce(u64::MAX), 30_000, 0));

        for value in 0..10_000 {
            assert!(window.insert(&device, &nonce(value), 30_000 + value, value));
        }

        assert!(!window.insert(&device, &nonce(u64::MAX), 30_000, 9_999));
    }

    #[test]
    fn a_full_window_refuses_new_nonces_until_entries_expire() {
        let mut window = ReplayWindow::default();
        let device = [1; 16];
        for value in 0..CAPACITY as u64 {
            assert!(window.insert(&device, &nonce(value), 1_000, 0));
        }

        assert!(!window.insert(&device, &nonce(u64::MAX), 1_000, 0));
        assert!(window.insert(&device, &nonce(u64::MAX), 2_001, 1_001));
        assert_eq!(window.seen.len(), 1);
    }

    #[test]
    fn the_same_nonce_from_two_devices_does_not_collide() {
        let mut window = ReplayWindow::default();
        let nonce = [5; 16];

        assert!(window.insert(&[1; 16], &nonce, 1_000, 0));
        assert!(window.insert(&[2; 16], &nonce, 1_000, 0));
        assert!(!window.insert(&[1; 16], &nonce, 1_000, 0));
    }
}
