#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoProgressSnapshot {
    pub done: u32,
    pub total: u32,
    pub phase: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedConsoleSegment {
    pub line: String,
    pub snapshot: Option<CargoProgressSnapshot>,
}

pub fn drain_console_segments(pending: &mut String, mut on_segment: impl FnMut(&str)) {
    while let Some(idx) = pending.find(|c| c == '\n' || c == '\r') {
        let segment = pending[..idx].to_string();
        pending.drain(..=idx);
        on_segment(&segment);
    }
}

pub fn sanitize_console_line(raw: &str) -> String {
    let mut sanitized = String::with_capacity(raw.len());
    #[derive(Copy, Clone)]
    enum AnsiState {
        None,
        Escape,
        Csi,
    }
    let mut state = AnsiState::None;

    for ch in raw.chars() {
        match state {
            AnsiState::None => {
                if ch == '\u{1b}' {
                    state = AnsiState::Escape;
                } else if !ch.is_control() {
                    sanitized.push(ch);
                }
            }
            AnsiState::Escape => {
                if ch == '[' {
                    state = AnsiState::Csi;
                } else if ('@'..='~').contains(&ch) {
                    state = AnsiState::None;
                }
            }
            AnsiState::Csi => {
                if ('@'..='~').contains(&ch) {
                    state = AnsiState::None;
                }
            }
        }
    }

    sanitized.trim().to_string()
}

pub fn parse_cargo_progress_line(line: &str) -> Option<CargoProgressSnapshot> {
    if !line.contains("Building [") {
        return None;
    }

    let bar_end = line.rfind(']')?;
    let tail = line.get(bar_end + 1..)?.trim();

    let mut tail_parts = tail.splitn(2, ':');
    let ratio = tail_parts.next()?.trim();
    let phase = tail_parts.next().unwrap_or("").trim().to_string();

    let mut ratio_parts = ratio.split('/');
    let done = ratio_parts.next()?.trim().parse::<u32>().ok()?;
    let total = ratio_parts.next()?.trim().parse::<u32>().ok()?;

    if total == 0 || done > total {
        return None;
    }

    Some(CargoProgressSnapshot { done, total, phase })
}

pub fn parse_console_segment(raw_segment: &str) -> Option<ParsedConsoleSegment> {
    let line = sanitize_console_line(raw_segment);
    if line.is_empty() {
        return None;
    }
    let snapshot = parse_cargo_progress_line(&line);
    Some(ParsedConsoleSegment { line, snapshot })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn sanitize_never_returns_control_chars(raw in ".*") {
            let sanitized = sanitize_console_line(&raw);
            prop_assert!(sanitized.chars().all(|ch| !ch.is_control()));
        }
    }

    #[test]
    fn parse_progress_line_extracts_done_total_and_phase() {
        let parsed = parse_cargo_progress_line("Building [=============>      ] 91/236: plugin-alt-tab")
            .expect("progress should parse");
        assert_eq!(parsed.done, 91);
        assert_eq!(parsed.total, 236);
        assert_eq!(parsed.phase, "plugin-alt-tab");
    }

    #[test]
    fn parse_progress_line_rejects_non_progress_text() {
        assert!(parse_cargo_progress_line("Compiling serde v1.0.228").is_none());
        assert!(parse_cargo_progress_line("Finished dev [unoptimized]").is_none());
    }
}
