const PREFIX_LOCK: usize = 10;
const MAX_DISTANCE: usize = 3;
const MIN_LENGTH_SLACK: usize = 1;
const MAX_LENGTH_SLACK: usize = 2;
const MAX_GAP: usize = 3;

pub fn marker_close_tolerant(text: &str, marker: &str) -> bool {
    if marker_close(text, marker) {
        return true;
    }
    let Some(index) = marker.rfind('_') else {
        return false;
    };
    let prefix = &marker[..=index];
    let token = &marker[index + 1..];
    if prefix.is_empty() || token.is_empty() {
        return false;
    }
    let mut cursor = 0usize;
    while let Some(found) = text[cursor..].find(prefix) {
        let start = cursor + found;
        if let Some(rest) = skip_conjunction(&text[start + prefix.len()..]) {
            if tail_matches(rest, token) {
                return true;
            }
        }
        cursor = start + prefix.len();
    }
    false
}

fn skip_conjunction(tail: &str) -> Option<&str> {
    let mut offset = 0usize;
    let mut junk = 0usize;
    for ch in tail.chars() {
        if ch.is_ascii_whitespace() || (ch.is_ascii() && !ch.is_alphanumeric()) {
            junk += 1;
            if junk > 3 {
                return None;
            }
            offset += ch.len_utf8();
        } else {
            break;
        }
    }
    let rest = &tail[offset..];
    for word in ["and", "or"] {
        if rest.len() >= word.len() && rest[..word.len()].eq_ignore_ascii_case(word) {
            let after = &rest[word.len()..];
            if after
                .chars()
                .next()
                .is_none_or(|ch| !ch.is_ascii_alphanumeric())
            {
                let mut inner = offset + word.len();
                for ch in after.chars() {
                    if ch.is_ascii_whitespace() || (ch.is_ascii() && !ch.is_alphanumeric()) {
                        inner += ch.len_utf8();
                        if inner - offset - word.len() > 3 {
                            return None;
                        }
                    } else {
                        break;
                    }
                }
                return Some(&tail[inner..]);
            }
        }
    }
    None
}

pub fn marker_close(text: &str, marker: &str) -> bool {
    let Some(index) = marker.rfind('_') else {
        return false;
    };
    let prefix = &marker[..=index];
    let token = &marker[index + 1..];
    if prefix.is_empty() || token.is_empty() {
        return false;
    }
    let mut cursor = 0usize;
    while let Some(found) = text[cursor..].find(prefix) {
        let start = cursor + found;
        if tail_matches(&text[start + prefix.len()..], token) {
            return true;
        }
        cursor = start + prefix.len();
    }
    false
}

fn tail_matches(tail: &str, token: &str) -> bool {
    if tail.starts_with(token) {
        return true;
    }
    let spaces = tail
        .chars()
        .take(4)
        .take_while(|ch| ch.is_ascii_whitespace())
        .count();
    if (1..=3).contains(&spaces) && tail[spaces..].starts_with(token) {
        return true;
    }
    let Some(candidate) = collect_candidate(tail, token.len()) else {
        return false;
    };
    candidate == token || candidate_close(&candidate, token)
}

fn collect_candidate(tail: &str, token_len: usize) -> Option<String> {
    let min_len = token_len.saturating_sub(MIN_LENGTH_SLACK);
    let max_len = token_len + MAX_LENGTH_SLACK;
    let mut candidate = String::new();
    let mut gap = 0usize;
    for ch in tail.chars() {
        if ch.is_ascii_alphanumeric() {
            gap = 0;
            if candidate.len() < max_len {
                candidate.push(ch);
            }
        } else if ch.is_ascii_whitespace() {
            gap += 1;
            if gap > MAX_GAP {
                return None;
            }
        } else {
            gap = 0;
        }
    }
    (candidate.len() >= min_len).then_some(candidate)
}

fn candidate_close(candidate: &str, token: &str) -> bool {
    if token.len() < PREFIX_LOCK {
        return false;
    }
    let lock = PREFIX_LOCK.min(token.len());
    let min_length = token.len() - MIN_LENGTH_SLACK;
    let max_length = token.len() + MAX_LENGTH_SLACK;
    for length in min_length..=max_length {
        let Some(window) = candidate.get(..length) else {
            break;
        };
        if !window.starts_with(&token[..lock]) {
            continue;
        }
        if bounded_edit_distance(window, token, MAX_DISTANCE).is_some() {
            return true;
        }
    }
    false
}

fn bounded_edit_distance(left: &str, right: &str, limit: usize) -> Option<usize> {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    if left.len().abs_diff(right.len()) > limit {
        return None;
    }
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0usize; right.len() + 1];
    for (i, left_char) in left.iter().enumerate() {
        current[0] = i + 1;
        let mut row_min = current[0];
        for (j, right_char) in right.iter().enumerate() {
            let substitution = previous[j] + usize::from(left_char != right_char);
            current[j + 1] = (previous[j + 1] + 1).min(current[j] + 1).min(substitution);
            row_min = row_min.min(current[j + 1]);
        }
        if row_min > limit {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    (previous[right.len()] <= limit).then_some(previous[right.len()])
}

#[cfg(test)]
mod tests {
    use super::{marker_close, marker_close_tolerant};

    #[test]
    fn mangled_token_within_three_edits_matches() {
        let marker = "QOL_BRIDGE_DONE_4aab033102f7a21f7322";
        assert!(marker_close(
            "done\nQOL_BRIDGE_DONE_4aab0331027f21a7322",
            marker
        ));
        assert!(marker_close(
            "done\nQOL_BRIDGE_DONE_4aab033102f7a21f7322",
            marker
        ));
        assert!(marker_close(
            "done\nQOL_BRIDGE_DONE_4aab033102f7a21f7322 report written",
            marker
        ));
        assert!(marker_close("QOL_BRIDGE_DONE_4aab033102f7a21f7399", marker));
        assert!(marker_close("QOL_BRIDGE_DONE_4aab033102f7a21f7999", marker));
    }

    #[test]
    fn four_or_more_edits_stay_rejected() {
        let marker = "QOL_BRIDGE_DONE_4aab033102f7a21f7322";
        assert!(!marker_close(
            "QOL_BRIDGE_DONE_4aab033102f7a21f9999",
            marker
        ));
        assert!(!marker_close("QOL_BRIDGE_DONE_4aab033102f7a219999", marker));
        assert!(!marker_close("QOL_BRIDGE_DONE_4aab03310299999999", marker));
    }

    #[test]
    fn reflow_and_line_wraps_normalize_away() {
        let marker = "QOL_BRIDGE_DONE_4aab033102f7a21f7322";
        assert!(marker_close(
            "QOL_BRIDGE_DONE_4aab0331\n02f7a21f7322",
            marker
        ));
        assert!(marker_close(
            "QOL_BRIDGE_DONE_4aab033102f7a2 1f7322",
            marker
        ));
        assert!(marker_close(
            "QOL_BRIDGE_DONE_ 4aab033102f7a21f7322",
            marker
        ));
    }

    #[test]
    fn split_echo_and_foreign_markers_never_match() {
        let marker = "QOL_BRIDGE_DONE_4aab033102f7a21f7322";
        assert!(!marker_close(
            "Completion fragments: `QOL_BRIDGE_DONE_` and `4aab033102f7a21f7322`.",
            marker
        ));
        assert!(!marker_close(
            "QOL_BRIDGE_DONE_99999999999999999999",
            marker
        ));
    }

    #[test]
    fn a_mangle_inside_the_locked_prefix_stays_rejected() {
        let marker = "QOL_BRIDGE_DONE_4aab033102f7a21f7322";
        assert!(!marker_close(
            "QOL_BRIDGE_DONE_4Xab033102f7a21f7322",
            marker
        ));
    }

    #[test]
    fn exact_joins_short_gaps_and_wide_gaps_keep_their_contract() {
        let marker = "QOL_BRIDGE_DONE_abc123xyz";
        assert!(marker_close("done\nQOL_BRIDGE_DONE_abc123xyz", marker));
        assert!(marker_close("done\nQOL_BRIDGE_DONE_ abc123xyz", marker));
        assert!(marker_close("done\nQOL_BRIDGE_DONE_\nabc123xyz", marker));
        assert!(marker_close("done\nQOL_BRIDGE_DONE_   abc123xyz", marker));
        assert!(!marker_close("QOL_BRIDGE_DONE_    abc123xyz", marker));
        assert!(!marker_close("QOL_BRIDGE_DONE_ and abc123xyz", marker));
        assert!(!marker_close("QOL_BRIDGE_DONE_ AND abc123xyz", marker));
        assert!(!marker_close("done\nQOL_BRIDGE_DONE_other987", marker));
    }

    #[test]
    fn empty_token_or_missing_underscore_never_matches() {
        assert!(!marker_close(
            "anything QOL_BRIDGE_DONE_",
            "QOL_BRIDGE_DONE_"
        ));
        assert!(!marker_close("QOL_BRIDGE_DONE_abc", "no_underscore"));
        assert!(!marker_close("abc", "QOL_BRIDGE_DONE_abc"));
        assert!(!marker_close("", "QOL_BRIDGE_DONE_abc"));
        assert!(!marker_close("QOL_BRIDGE_DONE_abc", ""));
    }

    #[test]
    fn tolerant_matching_accepts_the_literal_fragment_instruction() {
        let marker = "QOL_BRIDGE_DONE_4aab033102f7a21f7322";
        assert!(marker_close_tolerant(
            "Completion fragments: `QOL_BRIDGE_DONE_` and `4aab033102f7a21f7322`.",
            marker
        ));
        assert!(marker_close_tolerant(
            "QOL_BRIDGE_DONE_ and 4aab033102f7a21f7322",
            marker
        ));
        assert!(marker_close_tolerant(
            "QOL_BRIDGE_DONE_ AND 4aab033102f7a21f7322",
            marker
        ));
        assert!(marker_close_tolerant(
            "QOL_BRIDGE_DONE_ or 4aab033102f7a21f7322",
            marker
        ));
        assert!(marker_close_tolerant(
            "QOL_BRIDGE_DONE_ and\t4aab033102f7a21f7322",
            marker
        ));
        assert!(marker_close_tolerant(
            "QOL_BRIDGE_DONE_4aab033102f7a21f7322",
            marker
        ));
    }

    #[test]
    fn tolerant_matching_stays_bounded_and_foreign_markers_still_fail() {
        let marker = "QOL_BRIDGE_DONE_4aab033102f7a21f7322";
        assert!(!marker_close_tolerant(
            "QOL_BRIDGE_DONE_ and 4aab033102f7a21f9999",
            marker
        ));
        assert!(!marker_close_tolerant(
            "QOL_BRIDGE_DONE_ andromeda 4aab033102f7a21f7322",
            marker
        ));
        assert!(!marker_close_tolerant(
            "QOL_BRIDGE_DONE_    and 4aab033102f7a21f7322",
            marker
        ));
        assert!(!marker_close_tolerant(
            "QOL_BRIDGE_DONE_ and     4aab033102f7a21f7322",
            marker
        ));
        assert!(!marker_close_tolerant(
            "QOL_BRIDGE_DONE_99999999999999999999 and 4aab033102f7a21f7322",
            marker
        ));
    }
}
