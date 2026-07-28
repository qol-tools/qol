use proptest::prelude::*;

mod common;
use common::config;

struct FrequencyEntry {
    count: u32,
    last_accessed: u64,
}

fn effective_count(entry: &FrequencyEntry, now: u64, half_life_days: f64) -> f64 {
    let days_elapsed = now.saturating_sub(entry.last_accessed) as f64 / 86400.0;
    let decay = (-days_elapsed * 0.693 / half_life_days).exp();
    entry.count as f64 * decay
}

const SECS_PER_DAY: u64 = 86400;

proptest! {
    #![proptest_config(config())]

    #[test]
    fn prop_decay_non_negative(
        count in 0u32..10000,
        days_ago in 0u64..365,
        half_life in 1.0f64..90.0
    ) {
        let now = 1_000_000_000u64;
        let entry = FrequencyEntry { count, last_accessed: now - days_ago * SECS_PER_DAY };
        let result = effective_count(&entry, now, half_life);
        prop_assert!(result >= 0.0, "effective_count was negative: {}", result);
    }

    #[test]
    fn prop_decay_zero_count_is_zero(
        days_ago in 0u64..365,
        half_life in 1.0f64..90.0
    ) {
        let now = 1_000_000_000u64;
        let entry = FrequencyEntry { count: 0, last_accessed: now - days_ago * SECS_PER_DAY };
        let result = effective_count(&entry, now, half_life);
        prop_assert!(
            result.abs() < f64::EPSILON,
            "Zero count should give zero, got {}", result
        );
    }

    #[test]
    fn prop_decay_zero_elapsed_equals_count(
        count in 1u32..10000,
        half_life in 1.0f64..90.0
    ) {
        let now = 1_000_000_000u64;
        let entry = FrequencyEntry { count, last_accessed: now };
        let result = effective_count(&entry, now, half_life);
        prop_assert!(
            (result - count as f64).abs() < 0.01,
            "At zero elapsed, expected {} got {}", count, result
        );
    }

    #[test]
    fn prop_decay_halves_at_half_life(
        count in 1u32..10000,
        half_life in 1.0f64..90.0
    ) {
        let now = 1_000_000_000u64;
        let half_life_secs = (half_life * SECS_PER_DAY as f64) as u64;
        let entry = FrequencyEntry { count, last_accessed: now - half_life_secs };
        let result = effective_count(&entry, now, half_life);
        let expected = count as f64 / 2.0;
        let tolerance = expected * 0.01;
        prop_assert!(
            (result - expected).abs() < tolerance,
            "After one half-life: expected ~{:.2}, got {:.2}", expected, result
        );
    }

    #[test]
    fn prop_decay_monotonically_decreasing(
        count in 1u32..10000,
        days_a in 0u64..180,
        days_b in 0u64..180,
        half_life in 1.0f64..90.0
    ) {
        let now = 1_000_000_000u64;
        let earlier = now - days_a.max(days_b) * SECS_PER_DAY;
        let later = now - days_a.min(days_b) * SECS_PER_DAY;
        let entry_old = FrequencyEntry { count, last_accessed: earlier };
        let entry_new = FrequencyEntry { count, last_accessed: later };
        let old_score = effective_count(&entry_old, now, half_life);
        let new_score = effective_count(&entry_new, now, half_life);
        prop_assert!(
            new_score >= old_score,
            "More recent ({} days ago) scored {} < older ({} days ago) scored {}",
            days_a.min(days_b), new_score, days_a.max(days_b), old_score
        );
    }

    #[test]
    fn prop_decay_linear_in_count(
        count in 1u32..5000,
        days_ago in 0u64..365,
        half_life in 1.0f64..90.0
    ) {
        let now = 1_000_000_000u64;
        let entry_single = FrequencyEntry { count, last_accessed: now - days_ago * SECS_PER_DAY };
        let entry_double = FrequencyEntry { count: count * 2, last_accessed: now - days_ago * SECS_PER_DAY };
        let single = effective_count(&entry_single, now, half_life);
        let double = effective_count(&entry_double, now, half_life);
        prop_assert!(
            (double - single * 2.0).abs() < 0.01,
            "Double count ({}) should give double result: {} vs {}",
            count * 2, double, single * 2.0
        );
    }

    #[test]
    fn prop_decay_longer_half_life_slower(
        count in 1u32..10000,
        days_ago in 1u64..180,
        half_life_short in 1.0f64..30.0,
        half_life_long in 31.0f64..90.0
    ) {
        let now = 1_000_000_000u64;
        let entry = FrequencyEntry { count, last_accessed: now - days_ago * SECS_PER_DAY };
        let short_result = effective_count(&entry, now, half_life_short);
        let long_result = effective_count(&entry, now, half_life_long);
        prop_assert!(
            long_result >= short_result,
            "Longer half-life ({:.1}d) gave lower score {:.4} than shorter ({:.1}d) score {:.4}",
            half_life_long, long_result, half_life_short, short_result
        );
    }

    #[test]
    fn prop_decay_clock_skew_does_not_panic(
        count in 0u32..10000,
        now in 0u64..1_000_000,
        last_accessed in 0u64..1_000_000,
        half_life in 1.0f64..90.0
    ) {
        let entry = FrequencyEntry { count, last_accessed };
        let result = effective_count(&entry, now, half_life);
        prop_assert!(result.is_finite(), "Result should be finite, got {}", result);
    }
}
