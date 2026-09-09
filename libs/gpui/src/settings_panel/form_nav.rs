#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Up,
    Down,
    Tab,
    Activate,
    CommitEdit,
    Backspace,
    Insert(String),
    Close,
    CancelEdit,
}

pub fn intent(key: &str, key_char: Option<&str>, editing: bool) -> Option<Intent> {
    if editing {
        return match key {
            "enter" | "return" => Some(Intent::CommitEdit),
            "escape" => Some(Intent::CancelEdit),
            "backspace" => Some(Intent::Backspace),
            _ => key_char.map(|ch| Intent::Insert(ch.to_string())),
        };
    }
    match key {
        "up" => Some(Intent::Up),
        "down" => Some(Intent::Down),
        "tab" => Some(Intent::Tab),
        "enter" | "return" | "space" => Some(Intent::Activate),
        "escape" => Some(Intent::Close),
        _ => None,
    }
}

pub fn adjacent_visible_row(visible: &[usize], selected: usize, direction: isize) -> usize {
    let Some(position) = visible.iter().position(|index| *index == selected) else {
        return visible.first().copied().unwrap_or(0);
    };
    let next = if direction < 0 {
        position.saturating_sub(1)
    } else {
        (position + 1).min(visible.len() - 1)
    };
    visible[next]
}

pub fn wrapping_visible_row(visible: &[usize], selected: usize, direction: isize) -> usize {
    let Some(position) = visible.iter().position(|index| *index == selected) else {
        return visible.first().copied().unwrap_or(0);
    };
    let next = if direction < 0 {
        position.checked_sub(1).unwrap_or(visible.len() - 1)
    } else {
        (position + 1) % visible.len()
    };
    visible[next]
}

#[derive(Debug, PartialEq)]
pub enum EscapeStep {
    CloseFilter,
    PopCard,
    AscendRail,
    Dismiss,
}

pub fn escape_step(depth: usize, filter_open: bool, rail_can_ascend: bool) -> EscapeStep {
    if filter_open {
        return EscapeStep::CloseFilter;
    }
    if depth > 0 {
        return EscapeStep::PopCard;
    }
    if rail_can_ascend {
        return EscapeStep::AscendRail;
    }
    EscapeStep::Dismiss
}

#[cfg(test)]
mod tests {
    use super::{adjacent_visible_row, wrapping_visible_row};

    #[test]
    fn wrapping_steps_cycle_through_every_visible_row() {
        let visible = [0, 1, 2, 3];
        assert_eq!(wrapping_visible_row(&visible, 3, 1), 0);
        assert_eq!(wrapping_visible_row(&visible, 0, -1), 3);
        assert_eq!(wrapping_visible_row(&visible, 1, 1), 2);
        assert_eq!(wrapping_visible_row(&visible, 2, -1), 1);
    }

    #[test]
    fn adjacent_steps_saturate_at_both_ends() {
        let visible = [0, 1, 2, 3];
        assert_eq!(adjacent_visible_row(&visible, 0, -1), 0);
        assert_eq!(adjacent_visible_row(&visible, 3, 1), 3);
        assert_eq!(adjacent_visible_row(&visible, 1, 1), 2);
        assert_eq!(adjacent_visible_row(&visible, 2, -1), 1);
    }
}
