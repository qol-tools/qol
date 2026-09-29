use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation<T> {
    Value(T),
    Vanished,
    Unavailable,
    Reused,
    Truncated,
    BudgetExceeded,
    Unsupported,
    NotCaptured,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug)]
pub struct ProcessProvenance {
    pub pid: Observation<u32>,
    pub start_ticks: Observation<u64>,
    pub generation: Observation<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessStat {
    pub start_ticks: u64,
    pub state: char,
    pub ppid: i32,
    pub pgid: i32,
    pub sid: i32,
    pub proc_identity: ScopeIdentity,
}

#[derive(Clone, Debug)]
pub struct MemberObservation {
    pub node: usize,
    pub pid: u32,
    pub stat: Observation<ProcessStat>,
    pub identity_check: Observation<bool>,
    pub pidfd_alive: Observation<bool>,
}

#[derive(Clone, Debug)]
pub struct NodeObservation {
    pub index: usize,
    pub parent: Option<usize>,
    pub depth: usize,
    pub identity: Observation<ScopeIdentity>,
    pub populated: Observation<bool>,
    pub membership: Observation<usize>,
}

#[derive(Clone, Debug)]
pub struct ProcessTreeObservation {
    pub support: &'static str,
    pub duration: Duration,
    pub incomplete: bool,
    pub timing_perturbed: bool,
    pub budget_exceeded: bool,
    pub truncated: bool,
    pub scope: Observation<ScopeIdentity>,
    pub root_populated_before: Observation<bool>,
    pub root_populated_after: Observation<bool>,
    pub creator: ProcessProvenance,
    pub leader: ProcessProvenance,
    pub guardian: ProcessProvenance,
    pub nodes: Vec<NodeObservation>,
    pub members: Vec<MemberObservation>,
}

impl ProcessTreeObservation {
    pub fn unsupported() -> Self {
        let provenance = ProcessProvenance {
            pid: Observation::Unsupported,
            start_ticks: Observation::Unsupported,
            generation: Observation::Unsupported,
        };
        Self {
            support: "unsupported",
            duration: Duration::ZERO,
            incomplete: true,
            timing_perturbed: false,
            budget_exceeded: false,
            truncated: false,
            scope: Observation::Unsupported,
            root_populated_before: Observation::Unsupported,
            root_populated_after: Observation::Unsupported,
            creator: provenance.clone(),
            leader: provenance.clone(),
            guardian: provenance,
            nodes: Vec::new(),
            members: Vec::new(),
        }
    }
}
