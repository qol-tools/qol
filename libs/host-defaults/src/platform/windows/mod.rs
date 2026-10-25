mod blockers;

use anyhow::{bail, Result};

use crate::claims::{Ledger, ReleaseOutcome};
use crate::handler;
use crate::host::{HostDefaults, OpenerSpec};
use crate::report::{Blocker, Lifetime, Report, SlotRoute};
use crate::role::Role;

const REASON: &str = "not built on Windows yet";

pub(crate) struct Platform;

impl HostDefaults for Platform {
    fn probe(&self, role: Role) -> Report {
        Report::impossible(role, REASON)
    }

    fn slot_route(&self, _role: Role) -> Option<SlotRoute> {
        None
    }

    fn blockers(&self) -> Vec<Blocker> {
        blockers::current()
    }

    fn claim_handler(
        &self,
        _role: Role,
        _opener: &OpenerSpec,
        _plugin_id: &str,
        _lifetime: Lifetime,
        _ledger: &Ledger,
    ) -> Result<()> {
        bail!("{REASON}")
    }

    fn release(&self, role: Role, ledger: &Ledger) -> Result<ReleaseOutcome> {
        handler::release_role(role, ledger)
    }
}
