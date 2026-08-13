use std::collections::HashMap;

use crate::LineChange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OldLineFate {
    Matched(usize),
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewLineFate {
    CarriedFrom(usize),
    MorphedFrom(usize),
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionPlan {
    pub old: Vec<OldLineFate>,
    pub new: Vec<NewLineFate>,
}

impl TransitionPlan {
    pub fn between(old: &[LineChange], new: &[LineChange]) -> Self {
        let mut old_fates = vec![OldLineFate::Removed; old.len()];
        let mut new_fates = vec![NewLineFate::Added; new.len()];
        let mut old_keys: HashMap<u32, usize> = HashMap::new();
        for (index, line) in old.iter().enumerate() {
            if let Some(key) = line.old_line_no {
                old_keys.entry(key).or_insert(index);
            }
        }
        let mut matched_new = vec![false; new.len()];
        for (index, line) in new.iter().enumerate() {
            let Some(key) = line.new_line_no else {
                continue;
            };
            let Some(&old_index) = old_keys.get(&key) else {
                continue;
            };
            old_fates[old_index] = OldLineFate::Matched(index);
            new_fates[index] = if old[old_index].text == line.text {
                NewLineFate::CarriedFrom(old_index)
            } else {
                NewLineFate::MorphedFrom(old_index)
            };
            matched_new[index] = true;
            old_keys.remove(&key);
        }
        for old_index in 0..old.len() {
            if old_fates[old_index] != OldLineFate::Removed || old[old_index].old_line_no.is_some()
            {
                continue;
            }
            let Some(new_index) = (0..new.len()).find(|&new_index| {
                !matched_new[new_index]
                    && new[new_index].new_line_no.is_none()
                    && new[new_index].text == old[old_index].text
            }) else {
                continue;
            };
            old_fates[old_index] = OldLineFate::Matched(new_index);
            new_fates[new_index] = NewLineFate::CarriedFrom(old_index);
            matched_new[new_index] = true;
        }
        Self {
            old: old_fates,
            new: new_fates,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{LineChange, LineKind};

    use super::*;

    fn line(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str) -> LineChange {
        LineChange {
            kind,
            text: text.to_string(),
            token_spans: Vec::new(),
            old_line_no: old,
            new_line_no: new,
        }
    }

    #[test]
    fn empty_old_diff_marks_every_new_line_added() {
        let plan = TransitionPlan::between(&[], &[line(LineKind::Added, None, Some(1), "a")]);
        assert!(plan.old.is_empty());
        assert_eq!(plan.new, vec![NewLineFate::Added]);
    }

    #[test]
    fn empty_new_diff_marks_every_old_line_removed() {
        let plan = TransitionPlan::between(&[line(LineKind::Removed, Some(1), None, "a")], &[]);
        assert_eq!(plan.old, vec![OldLineFate::Removed]);
        assert!(plan.new.is_empty());
    }

    #[test]
    fn both_empty_is_a_trivial_plan() {
        let plan = TransitionPlan::between(&[], &[]);
        assert!(plan.old.is_empty());
        assert!(plan.new.is_empty());
    }

    #[test]
    fn identical_context_lines_carry_by_position() {
        let old = vec![
            line(LineKind::Context, Some(1), Some(1), "one"),
            line(LineKind::Context, Some(2), Some(2), "two"),
        ];
        let new = vec![
            line(LineKind::Context, Some(1), Some(1), "one"),
            line(LineKind::Context, Some(2), Some(2), "two"),
        ];
        let plan = TransitionPlan::between(&old, &new);
        assert_eq!(
            plan.old,
            vec![OldLineFate::Matched(0), OldLineFate::Matched(1)]
        );
        assert_eq!(
            plan.new,
            vec![NewLineFate::CarriedFrom(0), NewLineFate::CarriedFrom(1)]
        );
    }

    #[test]
    fn added_only_transition_slides_new_lines_in() {
        let old = vec![line(LineKind::Context, Some(1), Some(1), "keep")];
        let new = vec![
            line(LineKind::Context, Some(1), Some(1), "keep"),
            line(LineKind::Added, None, Some(2), "new"),
        ];
        let plan = TransitionPlan::between(&old, &new);
        assert_eq!(plan.old, vec![OldLineFate::Matched(0)]);
        assert_eq!(
            plan.new,
            vec![NewLineFate::CarriedFrom(0), NewLineFate::Added]
        );
    }

    #[test]
    fn removed_only_transition_ghosts_old_lines() {
        let old = vec![
            line(LineKind::Context, Some(1), Some(1), "keep"),
            line(LineKind::Removed, Some(2), None, "gone"),
        ];
        let new = vec![line(LineKind::Context, Some(1), Some(1), "keep")];
        let plan = TransitionPlan::between(&old, &new);
        assert_eq!(
            plan.old,
            vec![OldLineFate::Matched(0), OldLineFate::Removed]
        );
        assert_eq!(plan.new, vec![NewLineFate::CarriedFrom(0)]);
    }

    #[test]
    fn removed_in_old_matches_added_in_new_at_the_shared_position() {
        let old = vec![line(LineKind::Removed, Some(5), None, "line")];
        let new = vec![line(LineKind::Added, None, Some(5), "line")];
        let plan = TransitionPlan::between(&old, &new);
        assert_eq!(plan.old, vec![OldLineFate::Matched(0)]);
        assert_eq!(plan.new, vec![NewLineFate::CarriedFrom(0)]);
    }

    #[test]
    fn modified_lines_morph_in_place() {
        let old = vec![line(LineKind::Context, Some(3), Some(3), "before")];
        let new = vec![line(LineKind::Context, Some(3), Some(3), "after")];
        let plan = TransitionPlan::between(&old, &new);
        assert_eq!(plan.old, vec![OldLineFate::Matched(0)]);
        assert_eq!(plan.new, vec![NewLineFate::MorphedFrom(0)]);
    }

    #[test]
    fn old_added_falls_back_to_new_removed_by_text() {
        let old = vec![line(LineKind::Added, None, Some(9), "same text")];
        let new = vec![line(LineKind::Removed, Some(9), None, "same text")];
        let plan = TransitionPlan::between(&old, &new);
        assert_eq!(plan.old, vec![OldLineFate::Matched(0)]);
        assert_eq!(plan.new, vec![NewLineFate::CarriedFrom(0)]);
    }

    #[test]
    fn keyless_lines_do_not_match_against_keyed_lines() {
        let old = vec![line(LineKind::Added, None, Some(9), "same text")];
        let new = vec![line(LineKind::Context, Some(4), Some(4), "same text")];
        let plan = TransitionPlan::between(&old, &new);
        assert_eq!(plan.old, vec![OldLineFate::Removed]);
        assert_eq!(plan.new, vec![NewLineFate::Added]);
    }

    #[test]
    fn equal_text_at_different_positions_does_not_match() {
        let old = vec![line(LineKind::Removed, Some(2), None, "dup")];
        let new = vec![line(LineKind::Added, None, Some(7), "dup")];
        let plan = TransitionPlan::between(&old, &new);
        assert_eq!(plan.old, vec![OldLineFate::Removed]);
        assert_eq!(plan.new, vec![NewLineFate::Added]);
    }

    #[test]
    fn matching_is_one_to_one_on_the_new_side() {
        let old = vec![line(LineKind::Added, None, Some(5), "dup")];
        let new = vec![
            line(LineKind::Removed, Some(5), None, "dup"),
            line(LineKind::Removed, Some(6), None, "dup"),
        ];
        let plan = TransitionPlan::between(&old, &new);
        assert_eq!(plan.old, vec![OldLineFate::Matched(0)]);
        assert_eq!(
            plan.new,
            vec![NewLineFate::CarriedFrom(0), NewLineFate::Added],
            "one old line claims only the first keyless partner"
        );
    }

    #[test]
    fn matching_is_one_to_one_on_the_old_side() {
        let old = vec![
            line(LineKind::Added, None, Some(4), "dup"),
            line(LineKind::Added, None, Some(5), "dup"),
        ];
        let new = vec![line(LineKind::Removed, Some(4), None, "dup")];
        let plan = TransitionPlan::between(&old, &new);
        assert_eq!(
            plan.old,
            vec![OldLineFate::Matched(0), OldLineFate::Removed],
            "one new line claims only the first keyless partner"
        );
        assert_eq!(plan.new, vec![NewLineFate::CarriedFrom(0)]);
    }
}
