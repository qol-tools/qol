pub(crate) struct LaneScheduler {
    cursor: usize,
}

impl LaneScheduler {
    pub(crate) fn new() -> Self {
        Self { cursor: 0 }
    }

    pub(crate) fn plan(
        &mut self,
        selected: Option<u32>,
        visible: &[u32],
        in_flight: &[u32],
        background_slots: usize,
    ) -> Vec<u32> {
        let mut targets = Vec::new();
        if let Some(wid) = selected {
            if visible.contains(&wid) && !in_flight.contains(&wid) {
                targets.push(wid);
            }
        }
        let background_in_flight = in_flight.iter().filter(|w| Some(**w) != selected).count();
        let free_slots = background_slots.saturating_sub(background_in_flight);
        if free_slots == 0 || visible.is_empty() {
            return targets;
        }
        let start = self.cursor % visible.len();
        let mut picked = 0;
        for offset in 0..visible.len() {
            if picked == free_slots {
                break;
            }
            let idx = (start + offset) % visible.len();
            let wid = visible[idx];
            if Some(wid) == selected || in_flight.contains(&wid) || targets.contains(&wid) {
                continue;
            }
            targets.push(wid);
            picked += 1;
            self.cursor = start + offset + 1;
        }
        targets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_goes_first_then_backgrounds_round_robin_across_calls() {
        let mut scheduler = LaneScheduler::new();

        let first = scheduler.plan(Some(1), &[1, 2, 3, 4], &[], 2);
        assert_eq!(first, vec![1, 2, 3], "selected first, then two backgrounds");

        let saturated = scheduler.plan(Some(1), &[1, 2, 3, 4], &[1, 2, 3], 2);
        assert_eq!(saturated, Vec::<u32>::new(), "all lanes busy");

        let second = scheduler.plan(Some(1), &[1, 2, 3, 4], &[], 2);
        assert_eq!(
            second,
            vec![1, 4, 2],
            "cursor continues rotation instead of restarting"
        );
    }

    type Case<'a> = (&'a str, Option<u32>, &'a [u32], &'a [u32], usize, &'a [u32]);

    #[test]
    fn plan_respects_selection_and_capacity() {
        let cases: [Case; 6] = [
            (
                "selected in flight yields backgrounds only",
                Some(1),
                &[1, 2, 3],
                &[1],
                2,
                &[2, 3],
            ),
            (
                "one background in flight leaves one slot",
                Some(1),
                &[1, 2, 3, 4],
                &[2],
                2,
                &[1, 3],
            ),
            (
                "full background lanes leave selected only",
                Some(1),
                &[1, 2, 3, 4],
                &[2, 3],
                2,
                &[1],
            ),
            (
                "no selection fills backgrounds",
                None,
                &[5, 6, 7],
                &[],
                2,
                &[5, 6],
            ),
            (
                "closed selected window falls back to backgrounds",
                Some(9),
                &[1, 2],
                &[],
                2,
                &[1, 2],
            ),
            ("empty visible plans nothing", Some(1), &[], &[], 2, &[]),
        ];
        for (label, selected, visible, in_flight, slots, expected) in cases {
            let mut scheduler = LaneScheduler::new();
            let plan = scheduler.plan(selected, visible, in_flight, slots);
            assert_eq!(plan, expected, "{label}");
        }
    }

    #[test]
    fn single_visible_window_is_planned_once() {
        let mut scheduler = LaneScheduler::new();
        assert_eq!(scheduler.plan(Some(1), &[1], &[], 2), vec![1]);
    }

    #[test]
    fn cursor_survives_visible_set_shrinking() {
        let mut scheduler = LaneScheduler::new();
        let big: Vec<u32> = (1..=20).collect();
        scheduler.plan(None, &big, &[], 8);

        let plan = scheduler.plan(None, &[30, 31], &[], 2);
        assert_eq!(plan.len(), 2, "shrunken visible set still plans");
        for wid in &plan {
            assert!([30, 31].contains(wid), "plans only current wids, got {wid}");
        }
    }
}
