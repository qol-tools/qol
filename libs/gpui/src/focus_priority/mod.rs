mod store;

use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub use store::FocusClaim;
use store::{outranking_claim, ClaimStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusPriority {
    Launcher,
    AltTab,
}

impl FocusPriority {
    pub const ALL: [FocusPriority; 2] = [FocusPriority::Launcher, FocusPriority::AltTab];

    pub fn outranks(self, other: FocusPriority) -> bool {
        self > other
    }

    fn key(self) -> &'static str {
        match self {
            FocusPriority::Launcher => "launcher",
            FocusPriority::AltTab => "alt_tab",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Summon {
    Granted,
    Outranked(FocusClaim),
}

static HELD: Mutex<Option<FocusPriority>> = Mutex::new(None);

fn held() -> Option<FocusPriority> {
    *HELD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn set_held(priority: Option<FocusPriority>) {
    *HELD.lock().unwrap_or_else(PoisonError::into_inner) = priority;
}

fn own_pid() -> u32 {
    std::process::id()
}

fn outranking(store: &ClaimStore, mine: FocusPriority) -> Option<FocusClaim> {
    outranking_claim(
        mine,
        own_pid(),
        store.read_above(mine),
        SystemTime::now(),
        qol_process::is_pid_alive,
    )
}

pub fn summon(priority: FocusPriority, title: &str) -> Summon {
    let Some(store) = ClaimStore::runtime() else {
        set_held(Some(priority));
        return Summon::Granted;
    };
    if let Some(claim) = outranking(&store, priority) {
        qol_runtime::probe!(
            "FOCUS_PRIORITY",
            "step=outranked mine={priority:?} by={:?} pid={} title={title}",
            claim.priority,
            claim.pid
        );
        return Summon::Outranked(claim);
    }
    set_held(Some(priority));
    let written = store
        .write(&FocusClaim::new(
            priority,
            own_pid(),
            title,
            SystemTime::now(),
        ))
        .is_ok();
    qol_runtime::probe!(
        "FOCUS_PRIORITY",
        "step=claimed mine={priority:?} written={written} title={title}"
    );
    Summon::Granted
}

pub fn release(priority: FocusPriority) {
    let mut slot = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    if *slot == Some(priority) {
        *slot = None;
    }
    drop(slot);
    let removed = ClaimStore::runtime().is_some_and(|store| store.remove_own(priority, own_pid()));
    qol_runtime::probe!(
        "FOCUS_PRIORITY",
        "step=released mine={priority:?} removed={removed}"
    );
}

pub fn must_yield() -> bool {
    let Some(mine) = held() else {
        return false;
    };
    let Some(store) = ClaimStore::runtime() else {
        return false;
    };
    let Some(claim) = outranking(&store, mine) else {
        return false;
    };
    qol_runtime::probe!(
        "FOCUS_PRIORITY",
        "step=yield mine={mine:?} to={:?} pid={} title={}",
        claim.priority,
        claim.pid,
        claim.title
    );
    true
}
