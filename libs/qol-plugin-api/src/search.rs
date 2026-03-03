#[derive(Debug, Clone, PartialEq)]
pub struct FuzzyMatch {
    pub score: i32,
    pub positions: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedFuzzyQuery {
    query_orig: Vec<char>,
    query_lower: Vec<char>,
}

pub fn prepare_fuzzy_query(query: &str) -> PreparedFuzzyQuery {
    PreparedFuzzyQuery {
        query_orig: query.chars().collect(),
        query_lower: query.chars().flat_map(|c| c.to_lowercase()).collect(),
    }
}

pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let prepared = prepare_fuzzy_query(query);
    fuzzy_match_prepared(&prepared, candidate)
}

pub fn fuzzy_match_prepared(prepared: &PreparedFuzzyQuery, candidate: &str) -> Option<FuzzyMatch> {
    if prepared.query_orig.is_empty() {
        return Some(FuzzyMatch { score: 0, positions: vec![] });
    }

    let c_orig: Vec<char> = candidate.chars().collect();
    let c_lower: Vec<char> = candidate.to_lowercase().chars().collect();

    let greedy = score_pass(
        &prepared.query_lower,
        &prepared.query_orig,
        &c_orig,
        &c_lower,
        false,
    );
    let boundary = score_pass(
        &prepared.query_lower,
        &prepared.query_orig,
        &c_orig,
        &c_lower,
        true,
    );
    let contiguous = score_contiguous_pass(
        &prepared.query_lower,
        &prepared.query_orig,
        &c_orig,
        &c_lower,
    );
    let word_match = score_word_match_pass(
        &prepared.query_lower,
        &prepared.query_orig,
        &c_orig,
        &c_lower,
    );

    [greedy, boundary, contiguous, word_match]
        .into_iter()
        .flatten()
        .min_by_key(|m| m.score)
}

fn score_pass(
    query: &[char],
    query_orig: &[char],
    candidate: &[char],
    candidate_lower: &[char],
    prefer_boundary: bool,
) -> Option<FuzzyMatch> {
    let mut positions = Vec::with_capacity(query.len());
    let mut start = 0;

    for &qc in query {
        let pos = if prefer_boundary {
            find_boundary_match(qc, candidate, candidate_lower, start)
        } else {
            find_first_match(qc, candidate_lower, start)
        };

        match pos {
            Some(p) => {
                positions.push(p);
                start = p + 1;
            }
            None => return None,
        }
    }

    Some(FuzzyMatch {
        score: compute_score(&positions, candidate, query_orig),
        positions,
    })
}

fn score_contiguous_pass(
    query: &[char],
    query_orig: &[char],
    candidate: &[char],
    candidate_lower: &[char],
) -> Option<FuzzyMatch> {
    if query.len() > candidate_lower.len() {
        return None;
    }

    let mut best: Option<FuzzyMatch> = None;
    for start in 0..=candidate_lower.len() - query.len() {
        if !query
            .iter()
            .zip(candidate_lower[start..start + query.len()].iter())
            .all(|(q, c)| q == c)
        {
            continue;
        }

        let positions: Vec<usize> = (start..start + query.len()).collect();
        let m = FuzzyMatch {
            score: compute_score(&positions, candidate, query_orig),
            positions,
        };

        best = match best {
            Some(current) if current.score <= m.score => Some(current),
            _ => Some(m),
        };
    }

    best
}

fn score_word_match_pass(
    query: &[char],
    query_orig: &[char],
    candidate: &[char],
    candidate_lower: &[char],
) -> Option<FuzzyMatch> {
    if query.len() > candidate_lower.len() {
        return None;
    }

    let mut best: Option<FuzzyMatch> = None;
    for start in 0..=candidate_lower.len() - query.len() {
        if !query
            .iter()
            .zip(candidate_lower[start..start + query.len()].iter())
            .all(|(q, c)| q == c)
        {
            continue;
        }

        let end = start + query.len();
        let at_word_start = start == 0 || is_separator(candidate[start - 1]);
        let at_word_end = end == candidate_lower.len() || is_separator(candidate[end]);

        if !at_word_start || !at_word_end {
            continue;
        }

        let positions: Vec<usize> = (start..end).collect();
        let word_bonus = -10 * query.len() as i32;
        let m = FuzzyMatch {
            score: compute_score(&positions, candidate, query_orig) + word_bonus,
            positions,
        };

        best = match best {
            Some(current) if current.score <= m.score => Some(current),
            _ => Some(m),
        };
    }

    best
}

fn is_separator(c: char) -> bool {
    c == ' ' || c == '-' || c == '_' || c == '/'
}

fn find_first_match(query_char: char, candidate_lower: &[char], start: usize) -> Option<usize> {
    candidate_lower[start..]
        .iter()
        .position(|&c| c == query_char)
        .map(|p| p + start)
}

fn find_boundary_match(
    query_char: char,
    candidate: &[char],
    candidate_lower: &[char],
    start: usize,
) -> Option<usize> {
    let mut first = None;
    for i in start..candidate_lower.len() {
        if candidate_lower[i] == query_char {
            if first.is_none() {
                first = Some(i);
            }
            if is_boundary(candidate, i) {
                return Some(i);
            }
        }
    }
    first
}

fn is_boundary(chars: &[char], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = chars[idx - 1];
    let curr = chars[idx];
    prev == ' ' || prev == '-' || prev == '_' || prev == '/'
        || (curr.is_uppercase() && prev.is_lowercase())
}

fn compute_score(positions: &[usize], candidate: &[char], query_orig: &[char]) -> i32 {
    let mut score = 0i32;
    let query_len = query_orig.len();

    for (i, &pos) in positions.iter().enumerate() {
        let gap = if i == 0 {
            // When the first match lands on a word boundary, the distance
            // into the string barely matters — cap it so "Microsoft Teams"
            // isn't punished for having a long prefix before "Teams".
            if pos > 0 && is_boundary(candidate, pos) {
                pos.min(1)
            } else {
                pos
            }
        } else {
            pos - positions[i - 1] - 1
        };

        score += gap as i32 * 3;

        if i > 0 && gap == 0 {
            score -= 4;
        }

        if is_boundary(candidate, pos) {
            score -= 6;
        }

        if pos == 0 {
            score -= 8;
        }

        if i < query_orig.len() && candidate[pos] == query_orig[i] {
            score -= 2;
        }
    }

    if query_len > 1 && is_fully_contiguous(positions) {
        score -= 12 * query_len as i32;
    }

    score
}

fn is_fully_contiguous(positions: &[usize]) -> bool {
    positions.len() > 1 && positions.windows(2).all(|w| w[1] == w[0] + 1)
}

#[cfg(test)]
mod tests {
    use super::fuzzy_match;

    #[test]
    fn word_match_beats_contiguous_substring() {
        let vscode = fuzzy_match("code", "Visual Studio Code").unwrap();
        let xcode = fuzzy_match("code", "Xcode").unwrap();
        assert!(
            vscode.score < xcode.score,
            "Visual Studio Code ({}) should score better than Xcode ({})",
            vscode.score,
            xcode.score
        );
    }

    #[test]
    fn word_match_not_triggered_for_substring() {
        let xcode = fuzzy_match("code", "Xcode").unwrap();
        let vscode = fuzzy_match("code", "Visual Studio Code").unwrap();
        assert!(xcode.score > vscode.score);
    }

    #[test]
    fn word_boundary_match_beats_non_boundary_substring() {
        let teams = fuzzy_match("team", "Microsoft Teams").unwrap();
        let steam = fuzzy_match("team", "Steam").unwrap();
        assert!(
            teams.score < steam.score,
            "Microsoft Teams ({}) should score better than Steam ({})",
            teams.score,
            steam.score
        );
    }

    #[test]
    fn case_bonus_favors_exact_case() {
        let exact = fuzzy_match("Code", "Visual Studio Code").unwrap();
        let lower = fuzzy_match("code", "Visual Studio Code").unwrap();
        assert!(
            exact.score < lower.score,
            "Exact case ({}) should score better than lowercase ({})",
            exact.score,
            lower.score
        );
    }
}
