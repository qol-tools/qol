use super::{CommitFault, Storage};

impl Storage {
    pub(in crate::service::authority) fn fail_next(&mut self, fault: CommitFault) {
        if let Self::Persistent(store) = self {
            store.fail_next(fault);
        }
    }
}
