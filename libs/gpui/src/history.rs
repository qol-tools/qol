#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoHistory<T> {
    applied: Vec<T>,
    reverted: Vec<T>,
}

impl<T> UndoHistory<T> {
    pub fn new() -> Self {
        Self {
            applied: Vec::new(),
            reverted: Vec::new(),
        }
    }

    pub fn record(&mut self, edit: T) {
        self.applied.push(edit);
        self.reverted.clear();
    }

    pub fn undo(&mut self) {
        let Some(edit) = self.applied.pop() else {
            return;
        };
        self.reverted.push(edit);
    }

    pub fn redo(&mut self) {
        let Some(edit) = self.reverted.pop() else {
            return;
        };
        self.applied.push(edit);
    }

    pub fn applied(&self) -> &[T] {
        &self.applied
    }

    pub fn can_undo(&self) -> bool {
        !self.applied.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.reverted.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.applied.is_empty()
    }

    pub fn len(&self) -> usize {
        self.applied.len()
    }
}

impl<T> Default for UndoHistory<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::UndoHistory;
    use proptest::prelude::*;

    #[test]
    fn recording_after_undo_starts_a_new_branch() {
        let mut history = UndoHistory::new();
        history.record("one");
        history.record("two");
        history.undo();

        history.record("three");
        history.redo();

        assert_eq!(history.applied(), &["one", "three"]);
        assert!(!history.can_redo());
    }

    #[test]
    fn empty_history_commands_are_stable() {
        let mut history = UndoHistory::<u8>::new();

        history.undo();
        history.redo();

        assert!(history.is_empty());
        assert!(!history.can_undo());
        assert!(!history.can_redo());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn undo_and_redo_preserve_edit_order(
            edits in prop::collection::vec(any::<i16>(), 0..100),
            requested_undos in 0usize..150,
        ) {
            let mut history = UndoHistory::new();
            for edit in edits.iter().copied() {
                history.record(edit);
            }
            let undo_count = requested_undos.min(edits.len());
            for _ in 0..undo_count {
                history.undo();
            }

            prop_assert_eq!(
                history.applied(),
                &edits[..edits.len() - undo_count]
            );

            for _ in 0..undo_count {
                history.redo();
            }

            prop_assert_eq!(history.applied(), edits.as_slice());
            prop_assert!(!history.can_redo());
        }
    }
}
