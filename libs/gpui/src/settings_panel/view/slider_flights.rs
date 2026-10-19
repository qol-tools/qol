use std::collections::{HashMap, HashSet};
use std::hash::Hash;

pub(super) struct SliderFlights<K, D> {
    flying: HashSet<K>,
    waiting: HashMap<K, D>,
}

impl<K, D> Default for SliderFlights<K, D> {
    fn default() -> Self {
        Self {
            flying: HashSet::new(),
            waiting: HashMap::new(),
        }
    }
}

impl<K: Clone + Eq + Hash, D> SliderFlights<K, D> {
    pub(super) fn offer(&mut self, key: K, dispatch: D) -> Option<D> {
        if self.flying.contains(&key) {
            self.waiting.insert(key, dispatch);
            return None;
        }
        self.flying.insert(key);
        Some(dispatch)
    }

    pub(super) fn is_flying(&self, key: &K) -> bool {
        self.flying.contains(key)
    }

    pub(super) fn land(&mut self, key: &K) -> Option<D> {
        let next = self.waiting.remove(key);
        if next.is_none() {
            self.flying.remove(key);
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_move_flies_at_once() {
        let mut flights = SliderFlights::default();
        assert_eq!(flights.offer("volume", 10), Some(10));
    }

    #[test]
    fn moves_during_a_flight_wait_and_only_the_newest_survives() {
        let mut flights = SliderFlights::default();
        assert_eq!(flights.offer("volume", 10), Some(10));
        assert_eq!(flights.offer("volume", 20), None);
        assert_eq!(flights.offer("volume", 30), None);
        assert_eq!(flights.land(&"volume"), Some(30));
        assert_eq!(flights.offer("volume", 40), None);
        assert_eq!(flights.land(&"volume"), Some(40));
        assert_eq!(flights.land(&"volume"), None);
        assert_eq!(flights.offer("volume", 50), Some(50));
    }

    #[test]
    fn a_slider_flies_from_its_first_offer_until_nothing_waits() {
        let mut flights = SliderFlights::default();
        assert!(!flights.is_flying(&"volume"));
        assert_eq!(flights.offer("volume", 10), Some(10));
        assert!(flights.is_flying(&"volume"));
        assert_eq!(flights.offer("volume", 20), None);
        assert_eq!(flights.land(&"volume"), Some(20));
        assert!(flights.is_flying(&"volume"));
        assert_eq!(flights.land(&"volume"), None);
        assert!(!flights.is_flying(&"volume"));
    }

    #[test]
    fn each_slider_flies_on_its_own() {
        let mut flights = SliderFlights::default();
        assert_eq!(flights.offer("volume", 10), Some(10));
        assert_eq!(flights.offer("microphone", 70), Some(70));
        assert_eq!(flights.offer("volume", 20), None);
        assert_eq!(flights.land(&"microphone"), None);
        assert_eq!(flights.offer("microphone", 80), Some(80));
        assert_eq!(flights.land(&"volume"), Some(20));
    }
}
