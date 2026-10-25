mod blockers;
mod folder;
mod session;
mod slots;

use anyhow::Result;

use crate::claims::{Ledger, ReleaseOutcome};
use crate::handler;
use crate::host::{HostDefaults, OpenerSpec};
use crate::report::{Blocker, Lifetime, Report, SlotRoute};
use crate::role::Role;
use session::Session;

pub(crate) struct Platform;

impl HostDefaults for Platform {
    fn probe(&self, role: Role) -> Report {
        match role {
            Role::FolderOpen => folder::probe_open(&Session::current()),
            Role::FolderReveal => folder::probe_reveal(),
            Role::SlotLauncher
            | Role::SlotWindowSwitcher
            | Role::SlotScreenshot
            | Role::SlotClipboardHistory
            | Role::SlotDictation => slots::probe(&Session::current(), role),
        }
    }

    fn slot_route(&self, role: Role) -> Option<SlotRoute> {
        slots::route(&Session::current(), role)
    }

    fn blockers(&self) -> Vec<Blocker> {
        blockers::current(&Session::current())
    }

    fn claim_handler(
        &self,
        role: Role,
        opener: &OpenerSpec,
        plugin_id: &str,
        lifetime: Lifetime,
        ledger: &Ledger,
    ) -> Result<()> {
        folder::claim_open(
            &Session::current(),
            role,
            opener,
            plugin_id,
            lifetime,
            ledger,
        )
    }

    fn release(&self, role: Role, ledger: &Ledger) -> Result<ReleaseOutcome> {
        handler::release_role(role, ledger)
    }
}
