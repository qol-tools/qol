use std::collections::HashSet;

pub(crate) struct LaneScheduler {
    attempted: HashSet<u32>,
}

impl LaneScheduler {
    pub(crate) fn new() -> Self {
        Self {
            attempted: HashSet::new(),
        }
    }

    pub(crate) fn plan(
        &mut self,
        live: &[u32],
        visible: &[u32],
        in_flight: &[u32],
        background_slots: usize,
    ) -> Vec<u32> {
        let mut targets = Vec::new();
        for &wid in live {
            if visible.contains(&wid) && !in_flight.contains(&wid) {
                self.attempted.insert(wid);
                targets.push(wid);
            }
        }
        let background_in_flight = in_flight.iter().filter(|w| !live.contains(w)).count();
        let mut free_slots = background_slots.saturating_sub(background_in_flight);
        for &wid in visible {
            if free_slots == 0 {
                break;
            }
            if live.contains(&wid) || in_flight.contains(&wid) || !self.attempted.insert(wid) {
                continue;
            }
            targets.push(wid);
            free_slots -= 1;
        }
        targets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_selected_window_is_recaptured_after_one_pass() {
        let mut scheduler = LaneScheduler::new();
        let all = [1, 2, 3, 4];

        assert_eq!(scheduler.plan(&[1], &all, &[], 2), vec![1, 2, 3]);
        assert_eq!(scheduler.plan(&[1], &all, &[], 2), vec![1, 4]);
        assert_eq!(
            scheduler.plan(&[1], &all, &[], 2),
            vec![1],
            "backgrounds are captured once per show"
        );
    }

    #[test]
    fn every_live_window_is_recaptured_each_pass() {
        let mut scheduler = LaneScheduler::new();
        let all = [1, 2, 3];

        assert_eq!(scheduler.plan(&[1, 2], &all, &[], 2), vec![1, 2, 3]);
        assert_eq!(scheduler.plan(&[1, 2], &all, &[], 2), vec![1, 2]);
        assert_eq!(scheduler.plan(&[1, 2], &all, &[2], 2), vec![1]);
    }

    type Case<'a> = (&'a str, &'a [u32], &'a [u32], &'a [u32], usize, &'a [u32]);

    #[test]
    fn plan_respects_selection_and_capacity() {
        let cases: [Case; 6] = [
            (
                "selected in flight yields backgrounds only",
                &[1],
                &[1, 2, 3],
                &[1],
                2,
                &[2, 3],
            ),
            (
                "one background in flight leaves one slot",
                &[1],
                &[1, 2, 3, 4],
                &[2],
                2,
                &[1, 3],
            ),
            (
                "full background lanes leave selected only",
                &[1],
                &[1, 2, 3, 4],
                &[2, 3],
                2,
                &[1],
            ),
            (
                "no selection fills backgrounds",
                &[],
                &[5, 6, 7],
                &[],
                2,
                &[5, 6],
            ),
            (
                "closed selected window falls back to backgrounds",
                &[9],
                &[1, 2],
                &[],
                2,
                &[1, 2],
            ),
            ("empty visible plans nothing", &[1], &[], &[], 2, &[]),
        ];
        for (label, live, visible, in_flight, slots, expected) in cases {
            let mut scheduler = LaneScheduler::new();
            let plan = scheduler.plan(live, visible, in_flight, slots);
            assert_eq!(plan, expected, "{label}");
        }
    }

    #[test]
    fn single_visible_window_is_planned_once() {
        let mut scheduler = LaneScheduler::new();
        assert_eq!(scheduler.plan(&[1], &[1], &[], 2), vec![1]);
    }

    #[test]
    fn a_window_that_appears_mid_show_is_captured_once() {
        let mut scheduler = LaneScheduler::new();
        assert_eq!(scheduler.plan(&[], &[1, 2], &[], 2), vec![1, 2]);
        assert_eq!(scheduler.plan(&[], &[1, 2, 3], &[], 2), vec![3]);
    }
}
