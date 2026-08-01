use super::super::ProcessSnapshot;

pub(in super::super) fn process_snapshot() -> Option<ProcessSnapshot> {
    Some(ProcessSnapshot::default())
}
