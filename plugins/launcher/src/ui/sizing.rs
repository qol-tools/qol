use qol_gpui::theme::{TextStyle, RADIUS_CONTROL, RADIUS_TIGHT, TEXT_BODY};

const DISPLAY_FROM: f32 = 18.0;
const NAME_LINE: f32 = 1.2;
const SUMMARY_GAP: f32 = 6.0;
const SUMMARY_LINE: f32 = 16.0;
const CARD_AIR: f32 = 22.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppCardSize {
    pub height: f32,
    pub tile: f32,
    pub icon: f32,
    pub radius: f32,
    pub name: TextStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FileCardSize {
    pub height: f32,
    pub name: TextStyle,
}

pub fn app_card(strength: f32) -> AppCardSize {
    let t = strength.clamp(0.0, 1.0);
    let glyph = (20.0 + 20.0 * t).round();
    let tile = glyph + 8.0;
    let name = name_size(t);
    let text = name * NAME_LINE + SUMMARY_GAP + SUMMARY_LINE;
    AppCardSize {
        height: (text.max(tile) + CARD_AIR).round(),
        tile,
        icon: glyph - 4.0,
        radius: if tile > 30.0 {
            RADIUS_CONTROL
        } else {
            RADIUS_TIGHT
        },
        name: name_style(name),
    }
}

pub fn file_card(strength: f32) -> FileCardSize {
    let name = name_size(strength.clamp(0.0, 1.0));
    FileCardSize {
        height: (80.0 + 1.25 * name).round(),
        name: name_style(name),
    }
}

pub fn name_style(size: f32) -> TextStyle {
    if size >= DISPLAY_FROM {
        TextStyle::Heading
    } else if size >= TEXT_BODY {
        TextStyle::Name
    } else {
        TextStyle::ListName
    }
}

fn name_size(t: f32) -> f32 {
    ((13.5 + 6.5 * t) * 2.0).round() / 2.0
}

pub fn usage_strength(bonus: i32, top: i32) -> f32 {
    if top <= 0 {
        return 1.0;
    }
    (bonus as f32 / top as f32).clamp(0.0, 1.0)
}

pub fn rank_strength(rank: usize) -> f32 {
    (1.0 - 0.2 * rank as f32).max(0.0)
}

pub fn match_strength(name: &str, query: &str) -> f32 {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return 1.0;
    }
    let name = name.to_lowercase();
    let ratio = query.chars().count() as f32 / name.chars().count().max(1) as f32;
    let score = if name.starts_with(&query) {
        90.0 + 10.0 * ratio
    } else if name
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .skip(1)
        .any(|word| word.starts_with(&query))
    {
        72.0 + 10.0 * ratio
    } else if let Some(at) = name.find(&query) {
        (62.0 + 10.0 * ratio - name[..at].chars().count() as f32).max(50.0)
    } else {
        let Some(gaps) = fuzzy_gaps(&name, &query) else {
            return 0.0;
        };
        (48.0 + 10.0 * ratio - 4.0 * gaps as f32).max(30.0)
    };
    ((score.round() - 30.0) / 70.0).clamp(0.0, 1.0)
}

fn fuzzy_gaps(name: &str, query: &str) -> Option<usize> {
    let name: Vec<char> = name.chars().collect();
    let mut gaps = 0;
    let mut last: Option<usize> = None;
    for ch in query.chars() {
        let from = last.map_or(0, |at| at + 1);
        let found = from
            + name
                .get(from..)?
                .iter()
                .position(|candidate| *candidate == ch)?;
        if last.is_some_and(|at| found > at + 1) {
            gaps += 1;
        }
        last = Some(found);
    }
    Some(gaps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_word_contains_and_scattered_matches_shrink_in_that_order() {
        let prefix = match_strength("Terminal", "te");
        let word = match_strength("Date & Time", "ti");
        let inside = match_strength("XTerm", "te");
        let scattered = match_strength("System Administration", "sa");
        assert!(
            prefix > word && word > inside && inside > scattered,
            "{prefix} {word} {inside} {scattered}"
        );
        assert_eq!(match_strength("Terminal", "zz"), 0.0);
        assert_eq!(match_strength("Terminal", "Terminal"), 1.0);
    }

    #[test]
    fn matches_follow_the_approved_board() {
        assert_eq!(app_card(match_strength("Terminal", "te")).height, 68.0);
        assert_eq!(app_card(match_strength("XTerm", "te")).height, 64.0);
        let terminal = app_card(match_strength("Terminal", "te"));
        assert_eq!(
            (terminal.tile, terminal.icon, terminal.name),
            (46.0, 34.0, TextStyle::Heading)
        );
    }

    #[test]
    fn rows_and_cards_grow_from_the_floor_to_the_ceiling() {
        assert_eq!(
            app_card(0.0),
            AppCardSize {
                height: 60.0,
                tile: 28.0,
                icon: 16.0,
                radius: RADIUS_TIGHT,
                name: TextStyle::ListName,
            }
        );
        assert_eq!(
            app_card(1.0),
            AppCardSize {
                height: 70.0,
                tile: 48.0,
                icon: 36.0,
                radius: RADIUS_CONTROL,
                name: TextStyle::Heading,
            }
        );
        assert_eq!(file_card(1.0).height, 105.0);
        assert_eq!(file_card(0.0).height, 97.0);
    }

    #[test]
    fn usage_and_rank_scale_from_the_top_entry() {
        assert_eq!(usage_strength(50, 100), 0.5);
        assert_eq!(usage_strength(5, 0), 1.0);
        assert_eq!(rank_strength(0), 1.0);
        assert!((rank_strength(2) - 0.6).abs() < 1e-6);
        assert_eq!(rank_strength(9), 0.0);
    }
}
