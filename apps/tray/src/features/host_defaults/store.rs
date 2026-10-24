use anyhow::{Context, Result};
use qol_host_defaults::{ClaimRecord, Ledger, Lifetime};
use qol_host_fixes::residency::HostResidency;

const LEDGER_FILE: &str = "host-defaults.json";

pub(super) fn ledger() -> Result<Ledger> {
    let dir = qol_config::data_dir().context("no data directory for the host defaults ledger")?;
    Ok(Ledger::at(dir.join(LEDGER_FILE)))
}

pub(super) fn current_lifetime() -> Lifetime {
    lifetime_for(HostResidency::current())
}

pub(super) fn lifetime_for(residency: HostResidency) -> Lifetime {
    match residency {
        HostResidency::Resident => Lifetime::ResidentPolicy,
        HostResidency::Portable => Lifetime::PortableSession,
    }
}

pub(super) fn claims(ledger: &Ledger) -> Result<Vec<ClaimRecord>> {
    ledger.load()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residency_decides_the_lifetime() {
        assert_eq!(
            lifetime_for(HostResidency::Resident),
            Lifetime::ResidentPolicy
        );
        assert_eq!(
            lifetime_for(HostResidency::Portable),
            Lifetime::PortableSession
        );
    }
}
