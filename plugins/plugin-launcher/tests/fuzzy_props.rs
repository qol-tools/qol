use launcher::{fuzzy_match, FuzzyMatch};
use proptest::prelude::*;

mod common;
use common::config;

proptest! {
    #![proptest_config(config())]

    #[test]
    fn prop_subsequence_valid(
        query in "[a-zA-Z]{1,8}",
        candidate in "[a-zA-Z \\-_]{1,30}"
    ) {
        if let Some(m) = fuzzy_match(&query, &candidate) {
            let q_lower: Vec<char> = query.to_lowercase().chars().collect();
            let c_lower: Vec<char> = candidate.to_lowercase().chars().collect();

            prop_assert_eq!(
                m.positions.len(), q_lower.len(),
                "Position count {} != query length {}", m.positions.len(), q_lower.len()
            );

            for window in m.positions.windows(2) {
                prop_assert!(
                    window[0] < window[1],
                    "Positions not strictly increasing: {} >= {}", window[0], window[1]
                );
            }

            for (i, &pos) in m.positions.iter().enumerate() {
                prop_assert!(
                    pos < c_lower.len(),
                    "Position {} out of bounds (len {})", pos, c_lower.len()
                );
                prop_assert_eq!(
                    c_lower[pos], q_lower[i],
                    "Mismatch at position {}: candidate '{}' != query '{}'",
                    pos, c_lower[pos], q_lower[i]
                );
            }
        }
    }

    #[test]
    fn prop_superset_of_substring(
        query in "[a-z]{1,5}",
        prefix in "[a-z]{0,5}",
        suffix in "[a-z]{0,5}"
    ) {
        let candidate = format!("{}{}{}", prefix, query, suffix);
        let result = fuzzy_match(&query, &candidate);
        prop_assert!(
            result.is_some(),
            "Substring '{}' in '{}' should fuzzy-match", query, candidate
        );
    }

    #[test]
    fn prop_empty_query_matches_all(
        candidate in "[a-zA-Z ]{0,30}"
    ) {
        let result = fuzzy_match("", &candidate);
        prop_assert_eq!(
            result,
            Some(FuzzyMatch { score: 0, positions: vec![] }),
            "Empty query should match with score 0"
        );
    }

    #[test]
    fn prop_case_insensitive_score(
        query in "[a-z]{1,6}",
        candidate in "[a-zA-Z ]{2,20}"
    ) {
        let lower = fuzzy_match(&query.to_lowercase(), &candidate);
        let upper = fuzzy_match(&query.to_uppercase(), &candidate);

        match (lower, upper) {
            (Some(l), Some(u)) => {
                prop_assert_eq!(
                    l.score, u.score,
                    "Case mismatch: lower scored {}, upper scored {}", l.score, u.score
                );
                prop_assert_eq!(
                    l.positions, u.positions,
                    "Case mismatch: different positions"
                );
            }
            (None, None) => {}
            (l, u) => prop_assert!(
                false,
                "One case matched, other didn't: lower={:?}, upper={:?}", l, u
            ),
        }
    }

    #[test]
    fn prop_no_match_missing_char(
        base in "[a-e]{1,5}",
    ) {
        let candidate = base.replace('a', "").replace('b', "").replace('c', "");
        let query = format!("{}z", base);
        if !candidate.to_lowercase().contains('z') {
            let result = fuzzy_match(&query, &candidate);
            prop_assert!(
                result.is_none(),
                "Query '{}' should not match '{}' (missing 'z')", query, candidate
            );
        }
    }

    #[test]
    fn prop_contiguous_beats_scattered(
        query in "[a-z]{2,5}",
        pad_len in 1usize..5
    ) {
        let contiguous = format!("{}{}", query, "0".repeat(pad_len));
        let scattered: String = query.chars()
            .flat_map(|c| std::iter::once(c).chain(std::iter::repeat_n('0', pad_len)))
            .collect();

        let c_score = fuzzy_match(&query, &contiguous).unwrap().score;
        let s_score = fuzzy_match(&query, &scattered).unwrap().score;

        prop_assert!(
            c_score < s_score,
            "Contiguous '{}' scored {} >= scattered '{}' scored {}",
            contiguous, c_score, scattered, s_score
        );
    }

    #[test]
    fn prop_prefix_beats_interior(
        query in "[a-z]{1,5}",
        pad_len in 1usize..5
    ) {
        let prefix_candidate = format!("{}{}", query, "0".repeat(pad_len));
        let interior_candidate = format!("{}{}", "0".repeat(pad_len), query);

        let p_score = fuzzy_match(&query, &prefix_candidate).unwrap().score;
        let i_score = fuzzy_match(&query, &interior_candidate).unwrap().score;

        prop_assert!(
            p_score < i_score,
            "Prefix '{}' scored {} >= interior '{}' scored {}",
            prefix_candidate, p_score, interior_candidate, i_score
        );
    }

    #[test]
    fn prop_boundary_beats_non_boundary(
        query in "[a-z]{1,4}"
    ) {
        let chars: Vec<char> = query.chars().collect();
        let boundary_candidate: String = chars.iter()
            .enumerate()
            .flat_map(|(i, &c)| {
                if i == 0 { vec![c] } else { vec![' ', c] }
            })
            .collect();

        let non_boundary_candidate: String = chars.iter()
            .flat_map(|&c| vec!['0', c])
            .collect();

        let b_score = fuzzy_match(&query, &boundary_candidate).unwrap().score;
        let n_score = fuzzy_match(&query, &non_boundary_candidate).unwrap().score;

        prop_assert!(
            b_score < n_score,
            "Boundary '{}' scored {} >= non-boundary '{}' scored {}",
            boundary_candidate, b_score, non_boundary_candidate, n_score
        );
    }
}

#[test]
fn contiguous_word_match_beats_scattered_early_match() {
    let query = "code";
    let contiguous = "Visual Studio Code";
    let scattered = "Account details";

    let contiguous_score = fuzzy_match(query, contiguous).unwrap().score;
    let scattered_score = fuzzy_match(query, scattered).unwrap().score;

    assert!(
        contiguous_score < scattered_score,
        "Contiguous '{}' scored {} >= scattered '{}' scored {}",
        contiguous,
        contiguous_score,
        scattered,
        scattered_score
    );
}
