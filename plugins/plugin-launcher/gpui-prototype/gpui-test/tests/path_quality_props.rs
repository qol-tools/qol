use proptest::prelude::*;

mod common;
use common::config;

struct PathQualityConfig {
    depth_penalty: i32,
    penalize_hidden: bool,
}

fn score_path_quality(path: &str, config: &PathQualityConfig) -> i32 {
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
    penalty += (depth as i32) * config.depth_penalty;

    if config.penalize_hidden {
        let hidden_count = path.split('/')
            .filter(|p| p.starts_with('.') && *p != ".local")
            .count();
        penalty += (hidden_count as i32) * 500;
    }

    penalty
}

fn path_cfg(depth_penalty: i32, penalize_hidden: bool) -> PathQualityConfig {
    PathQualityConfig { depth_penalty, penalize_hidden }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn prop_penalty_non_negative(
        segments in prop::collection::vec("[a-z]{1,10}", 1..10),
        depth_penalty in 0i32..20,
        penalize_hidden in proptest::bool::ANY
    ) {
        let path = format!("/{}", segments.join("/"));
        let result = score_path_quality(&path, &path_cfg(depth_penalty, penalize_hidden));
        prop_assert!(result >= 0, "Penalty was negative: {} for path '{}'", result, path);
    }

    #[test]
    fn prop_standard_dirs_lower_than_non_standard(
        name in "[a-z]{3,12}",
        depth_penalty in 0i32..20
    ) {
        let cfg = path_cfg(depth_penalty, false);
        let standard = format!("/usr/share/applications/{}.desktop", name);
        let non_standard = format!("/opt/share/{}.desktop", name);
        let standard_score = score_path_quality(&standard, &cfg);
        let non_standard_score = score_path_quality(&non_standard, &cfg);
        prop_assert!(
            standard_score <= non_standard_score,
            "Standard path '{}' scored {} > non-standard '{}' scored {}",
            standard, standard_score, non_standard, non_standard_score
        );
    }

    #[test]
    fn prop_deeper_paths_score_higher(
        base_segments in prop::collection::vec("[a-z]{1,8}", 1..5),
        extra_segments in prop::collection::vec("[a-z]{1,8}", 1..5),
        depth_penalty in 1i32..20
    ) {
        let cfg = path_cfg(depth_penalty, false);
        let shallow = format!("/{}", base_segments.join("/"));
        let deep = format!("/{}/{}", base_segments.join("/"), extra_segments.join("/"));
        let shallow_score = score_path_quality(&shallow, &cfg);
        let deep_score = score_path_quality(&deep, &cfg);
        prop_assert!(
            deep_score >= shallow_score,
            "Deeper path '{}' scored {} < shallower '{}' scored {}",
            deep, deep_score, shallow, shallow_score
        );
    }

    #[test]
    fn prop_hidden_dirs_add_500_each(
        visible_segments in prop::collection::vec("[a-z]{1,8}", 1..5),
        hidden_count in 1usize..4
    ) {
        let cfg = path_cfg(0, true);
        let mut with_hidden = visible_segments.clone();
        for i in 0..hidden_count {
            with_hidden.insert(i.min(with_hidden.len()), format!(".hidden{}", i));
        }
        let visible_path = format!("/{}", visible_segments.join("/"));
        let hidden_path = format!("/{}", with_hidden.join("/"));
        let visible_score = score_path_quality(&visible_path, &cfg);
        let hidden_score = score_path_quality(&hidden_path, &cfg);
        let diff = hidden_score - visible_score;
        let expected_hidden_penalty = hidden_count as i32 * 500;
        prop_assert!(
            diff >= expected_hidden_penalty,
            "Expected at least {} penalty for {} hidden dirs, got diff {}",
            expected_hidden_penalty, hidden_count, diff
        );
    }

    #[test]
    fn prop_hidden_penalty_disabled_ignores_hidden(
        segments in prop::collection::vec("[a-z]{1,8}", 1..5)
    ) {
        let mut with_hidden = segments.clone();
        with_hidden.push(".secret".to_string());
        let visible_path = format!("/{}", segments.join("/"));
        let hidden_path = format!("/{}", with_hidden.join("/"));
        let enabled = score_path_quality(&hidden_path, &path_cfg(0, true));
        let disabled = score_path_quality(&hidden_path, &path_cfg(0, false));
        let visible_score = score_path_quality(&visible_path, &path_cfg(0, false));
        prop_assert!(
            enabled > disabled,
            "Enabling hidden penalty should increase score: enabled={}, disabled={}",
            enabled, disabled
        );
        let depth_only_diff = disabled - visible_score;
        prop_assert!(
            depth_only_diff < 500,
            "With penalty disabled, hidden dir should not add 500: diff={}",
            depth_only_diff
        );
    }

    #[test]
    fn prop_dot_local_not_penalized_as_hidden(
        name in "[a-z]{3,12}"
    ) {
        let cfg = path_cfg(0, true);
        let with_local = format!("/a/.local/share/applications/{}.desktop", name);
        let with_hidden = format!("/a/.config/share/applications/{}.desktop", name);
        let local_score = score_path_quality(&with_local, &cfg);
        let hidden_score = score_path_quality(&with_hidden, &cfg);
        prop_assert!(
            local_score < hidden_score,
            ".local path scored {} >= .config path scored {}",
            local_score, hidden_score
        );
    }

    #[test]
    fn prop_autostart_xdg_adds_penalty(
        name in "[a-z]{3,12}"
    ) {
        let cfg = path_cfg(0, false);
        let normal = format!("/etc/{}.desktop", name);
        let autostart = format!("/etc/autostart/{}.desktop", name);
        let xdg = format!("/etc/xdg/{}.desktop", name);
        let normal_score = score_path_quality(&normal, &cfg);
        let autostart_score = score_path_quality(&autostart, &cfg);
        let xdg_score = score_path_quality(&xdg, &cfg);
        prop_assert!(
            autostart_score > normal_score,
            "Autostart path should score higher: autostart={}, normal={}",
            autostart_score, normal_score
        );
        prop_assert!(
            xdg_score > normal_score,
            "Xdg path should score higher: xdg={}, normal={}",
            xdg_score, normal_score
        );
    }
}
