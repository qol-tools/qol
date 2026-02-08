use proptest::prelude::*;
use std::collections::HashMap;

mod common;
use common::config;

struct RankingConfig {
    half_life_days: f64,
    frequency_bonus: i32,
    prefer_apps: bool,
    penalize_hidden: bool,
    depth_penalty: i32,
    exact_bonus: i32,
    prefix_penalty: i32,
    contains_penalty: i32,
}

impl Default for RankingConfig {
    fn default() -> Self {
        Self {
            half_life_days: 7.0,
            frequency_bonus: 500,
            prefer_apps: true,
            penalize_hidden: true,
            depth_penalty: 2,
            exact_bonus: 0,
            prefix_penalty: 100,
            contains_penalty: 200,
        }
    }
}

struct FrequencyEntry {
    count: u32,
    last_accessed: u64,
}

struct FrequencyData {
    entries: HashMap<String, FrequencyEntry>,
}

impl Default for FrequencyData {
    fn default() -> Self {
        Self { entries: HashMap::new() }
    }
}

struct SearchResult {
    path: String,
    name: String,
}

fn effective_count(entry: &FrequencyEntry, now: u64, half_life_days: f64) -> f64 {
    let days_elapsed = now.saturating_sub(entry.last_accessed) as f64 / 86400.0;
    let decay = (-days_elapsed * 0.693 / half_life_days).exp();
    entry.count as f64 * decay
}

fn score_path_quality(path: &str, cfg: &RankingConfig) -> i32 {
    let mut penalty = 0i32;
    let standard_dirs = ["/usr/share/applications", "/usr/lib", ".local/share/applications"];
    let is_standard = standard_dirs.iter().any(|d| path.contains(d));
    if !is_standard {
        penalty += 50;
    }
    if path.contains("/autostart/") || path.contains("/xdg/") {
        penalty += 30;
    }
    let depth = path.matches('/').count();
    penalty += (depth as i32) * cfg.depth_penalty;
    if cfg.penalize_hidden {
        let hidden_count = path.split('/')
            .filter(|p| p.starts_with('.') && *p != ".local")
            .count();
        penalty += (hidden_count as i32) * 500;
    }
    penalty
}

fn calc_frequency_bonus(path: &str, freq: &FrequencyData, cfg: &RankingConfig) -> i32 {
    let now = 1_000_000_000u64;
    freq.entries.get(path)
        .map(|e| (effective_count(e, now, cfg.half_life_days) * cfg.frequency_bonus as f64) as i32)
        .unwrap_or(0)
}

fn score_result(r: &SearchResult, query: &str, freq: &FrequencyData, cfg: &RankingConfig) -> i32 {
    let name = r.name.to_lowercase();
    let q = query.to_lowercase();

    let match_penalty = if name == q { cfg.exact_bonus }
        else if name.starts_with(&q) { cfg.prefix_penalty }
        else if name.contains(&q) { cfg.contains_penalty }
        else { 300 };

    let type_penalty = if !cfg.prefer_apps || r.path.ends_with(".desktop") { 0 } else { 1000 };
    let path_penalty = score_path_quality(&r.path, cfg);
    let length_penalty = r.name.len() as i32;
    let frequency_bonus = calc_frequency_bonus(&r.path, freq, cfg);

    match_penalty + type_penalty + path_penalty + length_penalty - frequency_bonus
}

fn app(name: &str) -> SearchResult {
    SearchResult {
        path: format!("/usr/share/applications/{}.desktop", name.to_lowercase()),
        name: name.to_string(),
    }
}

const NOW: u64 = 1_000_000_000;

proptest! {
    #![proptest_config(config())]

    #[test]
    fn prop_exact_beats_prefix(
        base in "[a-z]{2,8}"
    ) {
        let cfg = RankingConfig::default();
        let freq = FrequencyData::default();
        let exact = app(&base);
        let prefix = app(&format!("{}x", base));
        let exact_score = score_result(&exact, &base, &freq, &cfg);
        let prefix_score = score_result(&prefix, &base, &freq, &cfg);
        prop_assert!(
            exact_score < prefix_score,
            "Exact '{}' scored {} >= prefix '{}x' scored {}",
            base, exact_score, base, prefix_score
        );
    }

    #[test]
    fn prop_prefix_beats_contains(
        base in "[a-z]{2,8}"
    ) {
        let cfg = RankingConfig::default();
        let freq = FrequencyData::default();
        let prefix = app(&format!("{}x", base));
        let contains = app(&format!("x{}", base));
        prop_assume!(prefix.name != contains.name);
        let prefix_score = score_result(&prefix, &base, &freq, &cfg);
        let contains_score = score_result(&contains, &base, &freq, &cfg);
        prop_assert!(
            prefix_score < contains_score,
            "Prefix '{}x' scored {} >= contains 'x{}' scored {}",
            base, prefix_score, base, contains_score
        );
    }

    #[test]
    fn prop_desktop_beats_non_desktop(
        name in "[a-z]{2,8}"
    ) {
        let cfg = RankingConfig::default();
        let freq = FrequencyData::default();
        let desktop = app(&name);
        let folder = SearchResult {
            path: format!("/usr/share/{}", name),
            name: name.clone(),
        };
        let desktop_score = score_result(&desktop, &name, &freq, &cfg);
        let folder_score = score_result(&folder, &name, &freq, &cfg);
        prop_assert!(
            desktop_score < folder_score,
            "Desktop '{}' scored {} >= folder scored {}",
            name, desktop_score, folder_score
        );
    }

    #[test]
    fn prop_shorter_name_wins_same_match(
        base in "[a-z]{2,6}",
        extra_len in 1usize..10
    ) {
        let cfg = RankingConfig::default();
        let freq = FrequencyData::default();
        let short_name = format!("{}a", base);
        let long_name = format!("{}a{}", base, "b".repeat(extra_len));
        let short = app(&short_name);
        let long = app(&long_name);
        let short_score = score_result(&short, &base, &freq, &cfg);
        let long_score = score_result(&long, &base, &freq, &cfg);
        prop_assert!(
            short_score <= long_score,
            "Shorter '{}' scored {} > longer '{}' scored {}",
            short_name, short_score, long_name, long_score
        );
    }

    #[test]
    fn prop_frequency_overcomes_match_gap(
        base in "[a-z]{2,8}",
        count in 20u32..100
    ) {
        let cfg = RankingConfig::default();
        let prefix = app(&format!("{}x", base));
        let contains = app(&format!("x{}", base));
        let mut freq = FrequencyData::default();
        freq.entries.insert(
            contains.path.clone(),
            FrequencyEntry { count, last_accessed: NOW },
        );
        let prefix_score = score_result(&prefix, &base, &FrequencyData::default(), &cfg);
        let contains_score = score_result(&contains, &base, &freq, &cfg);
        prop_assert!(
            contains_score < prefix_score,
            "Frequent contains 'x{}' (count={}) scored {} >= infrequent prefix '{}x' scored {}",
            base, count, contains_score, base, prefix_score
        );
    }

    #[test]
    fn prop_case_insensitive_same_score(
        name in "[a-z]{2,10}"
    ) {
        let cfg = RankingConfig::default();
        let freq = FrequencyData::default();
        let r = app(&name);
        let lower_score = score_result(&r, &name.to_lowercase(), &freq, &cfg);
        let upper_score = score_result(&r, &name.to_uppercase(), &freq, &cfg);
        prop_assert_eq!(
            lower_score, upper_score,
            "Case mismatch: lowercase query={}, uppercase query={}",
            lower_score, upper_score
        );
    }

    #[test]
    fn prop_unknown_path_gets_no_frequency_bonus(
        name in "[a-z]{2,8}",
        other_count in 1u32..100
    ) {
        let cfg = RankingConfig::default();
        let r = app(&name);
        let mut freq = FrequencyData::default();
        freq.entries.insert(
            "/some/other/path.desktop".to_string(),
            FrequencyEntry { count: other_count, last_accessed: NOW },
        );
        let with_freq = score_result(&r, &name, &freq, &cfg);
        let without_freq = score_result(&r, &name, &FrequencyData::default(), &cfg);
        prop_assert_eq!(
            with_freq, without_freq,
            "Unrelated frequency data affected score: with={}, without={}",
            with_freq, without_freq
        );
    }
}
