#[derive(Debug, Default, Clone)]
pub struct CargoProgressEstimator {
    baseline_done: Option<u32>,
    last_done: u32,
    last_elapsed_secs: f64,
    avg_unit_secs: Option<f64>,
    ratio: f64,
}

impl CargoProgressEstimator {
    pub fn update(&mut self, observed_done: u32, observed_total: u32, elapsed_secs: f64) -> (u8, u32, u32) {
        if let Some(result) = self.ensure_baseline(observed_done, observed_total, elapsed_secs) {
            return result;
        }
        let (done, total) = self.rebase(observed_done, observed_total);
        self.update_ema(done, elapsed_secs);
        let raw_ratio = (done as f64 / total as f64).clamp(0.0, 1.0);
        let time_ratio = self.compute_time_ratio(done, total, elapsed_secs, raw_ratio);
        let percent = self.advance_ratio(done, total, raw_ratio, time_ratio);
        (percent, done, total)
    }

    fn ensure_baseline(&mut self, observed_done: u32, observed_total: u32, elapsed_secs: f64) -> Option<(u8, u32, u32)> {
        if self.baseline_done.is_some() {
            return None;
        }
        if observed_done == 0 {
            self.last_elapsed_secs = elapsed_secs;
            return Some((0, 0, observed_total.max(1)));
        }
        let rebased = observed_done
            .saturating_sub(1)
            .min(observed_total.saturating_sub(1));
        self.baseline_done = Some(rebased);
        None
    }

    fn rebase(&self, observed_done: u32, observed_total: u32) -> (u32, u32) {
        let baseline = self.baseline_done.unwrap_or(0);
        let mut total = observed_total.saturating_sub(baseline);
        if total == 0 { total = 1; }
        let done = observed_done.saturating_sub(baseline).min(total);
        (done, total)
    }

    fn update_ema(&mut self, done: u32, elapsed_secs: f64) {
        let delta_done = done.saturating_sub(self.last_done);
        let delta_elapsed = (elapsed_secs - self.last_elapsed_secs).max(0.0);
        if delta_done > 0 && delta_elapsed > 0.0 {
            let sample_unit_secs = delta_elapsed / delta_done as f64;
            self.avg_unit_secs = Some(match self.avg_unit_secs {
                Some(previous) => (previous * 0.7) + (sample_unit_secs * 0.3),
                None => sample_unit_secs,
            });
        }
        self.last_done = done;
        self.last_elapsed_secs = elapsed_secs;
    }

    fn compute_time_ratio(&self, done: u32, total: u32, elapsed_secs: f64, raw_ratio: f64) -> f64 {
        let Some(avg_unit_secs) = self.avg_unit_secs else { return raw_ratio; };
        let remaining_units = total.saturating_sub(done) as f64;
        if remaining_units <= 0.0 {
            return 1.0;
        }
        let eta_secs = avg_unit_secs * remaining_units;
        if eta_secs <= 0.0 {
            return raw_ratio;
        }
        (elapsed_secs / (elapsed_secs + eta_secs)).clamp(0.0, 1.0)
    }

    fn advance_ratio(&mut self, done: u32, total: u32, raw_ratio: f64, time_ratio: f64) -> u8 {
        let mut ratio = (raw_ratio * 0.35) + (time_ratio * 0.65);
        if done < total {
            ratio = ratio.min(0.985);
        } else {
            ratio = 0.99;
        }
        ratio = ratio.max(self.ratio);
        self.ratio = ratio;
        let mut percent = (ratio * 99.0).round() as u8;
        if ratio > 0.0 { percent = percent.max(1); }
        percent.min(99)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn estimator_percent_is_monotonic(
            samples in prop::collection::vec((0u32..3000, 1u32..3000, 1u32..20_000), 1..400)
        ) {
            let mut estimator = CargoProgressEstimator::default();
            let mut elapsed_secs = 0.0f64;
            let mut last_percent = 0u8;

            for (done_raw, total_raw, delta_ms) in samples {
                let total = total_raw.max(1);
                let done = done_raw % (total + 1);
                elapsed_secs += delta_ms as f64 / 1000.0;
                let (percent, _, _) = estimator.update(done, total, elapsed_secs);
                prop_assert!(percent >= last_percent);
                prop_assert!(percent <= 99);
                last_percent = percent;
            }
        }

        #[test]
        fn estimator_is_deterministic(
            samples in prop::collection::vec((0u32..1000, 1u32..1000, 1u32..20_000), 1..220)
        ) {
            let run = |input: &Vec<(u32, u32, u32)>| -> Vec<u8> {
                let mut estimator = CargoProgressEstimator::default();
                let mut elapsed = 0.0f64;
                let mut out = Vec::with_capacity(input.len());
                for (done_raw, total_raw, delta_ms) in input {
                    let total = (*total_raw).max(1);
                    let done = *done_raw % (total + 1);
                    elapsed += *delta_ms as f64 / 1000.0;
                    let (percent, _, _) = estimator.update(done, total, elapsed);
                    out.push(percent);
                }
                out
            };

            let left = run(&samples);
            let right = run(&samples);
            prop_assert_eq!(left, right);
        }
    }

    #[test]
    fn estimator_rebases_initial_done_units() {
        let mut estimator = CargoProgressEstimator::default();
        let (p0, d0, t0) = estimator.update(91, 236, 0.20);
        assert_eq!(d0, 1);
        assert_eq!(t0, 146);
        assert!(p0 <= 2);

        let (p1, d1, t1) = estimator.update(92, 236, 0.45);
        assert_eq!(d1, 2);
        assert_eq!(t1, 146);
        assert!(p1 >= p0);
    }

    #[test]
    fn estimator_keeps_percent_monotonic_when_done_temporarily_regresses() {
        let mut estimator = CargoProgressEstimator::default();

        let (p0, _, _) = estimator.update(66, 264, 0.193);
        assert_eq!(p0, 1);

        let (p1, _, _) = estimator.update(3, 1389, 0.194);
        assert!(p1 >= p0);
    }
}
