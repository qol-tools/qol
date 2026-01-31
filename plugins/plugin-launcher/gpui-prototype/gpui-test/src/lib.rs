use gpui::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchAction {
    Open,
    Terminal,
    OpenFolder,
    CopyPath,
}

pub fn action_for_modifiers(ctrl: bool, shift: bool, alt: bool) -> LaunchAction {
    if ctrl {
        LaunchAction::Terminal
    } else if shift {
        LaunchAction::OpenFolder
    } else if alt {
        LaunchAction::CopyPath
    } else {
        LaunchAction::Open
    }
}

pub fn action_hint(ctrl: bool, shift: bool, alt: bool) -> Option<LaunchAction> {
    if ctrl || shift || alt {
        Some(action_for_modifiers(ctrl, shift, alt))
    } else {
        None
    }
}

pub fn action_label(action: LaunchAction) -> &'static str {
    match action {
        LaunchAction::Open => "Open",
        LaunchAction::Terminal => "Open in Terminal",
        LaunchAction::OpenFolder => "Open Folder",
        LaunchAction::CopyPath => "Copy Path",
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuzzyMatch {
    pub score: i32,
    pub positions: Vec<usize>,
}

pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return Some(FuzzyMatch { score: 0, positions: vec![] });
    }

    let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let c_orig: Vec<char> = candidate.chars().collect();
    let c_lower: Vec<char> = candidate.to_lowercase().chars().collect();

    let greedy = score_pass(&q, &c_orig, &c_lower, false);
    let boundary = score_pass(&q, &c_orig, &c_lower, true);

    match (greedy, boundary) {
        (Some(g), Some(b)) => Some(if g.score <= b.score { g } else { b }),
        (g, b) => g.or(b),
    }
}

fn score_pass(
    query: &[char],
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
        score: compute_score(&positions, candidate),
        positions,
    })
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

fn compute_score(positions: &[usize], candidate: &[char]) -> i32 {
    let mut score = 0i32;

    for (i, &pos) in positions.iter().enumerate() {
        let gap = if i == 0 {
            pos
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
    }

    score
}

pub fn open_window_with_focus<T, F>(
    cx: &mut App,
    options: WindowOptions,
    build: F,
) -> Result<WindowHandle<T>>
where
    T: Render + Focusable + 'static,
    F: FnOnce(&mut Window, &mut Context<T>) -> T + 'static,
{
    cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| build(window, cx));
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        view
    })
}
