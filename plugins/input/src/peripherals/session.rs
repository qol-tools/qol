use anyhow::{Context, Result};
use qol_host_fixes::residency::HostResidency;
use qol_host_session::{
    HostMutation, Lifetime, MutationId, Residency, RestoreReport, SessionSnapshot, SessionStore,
};
use serde::{Deserialize, Serialize};

use super::{platform, Baseline, Setting};

const LEDGER: &str = "host-session";
const RECORD_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    schema_version: u32,
    setting: String,
    baseline: Baseline,
}

impl SessionSnapshot for Record {
    const SCHEMA_VERSION: u32 = RECORD_SCHEMA_VERSION;
    const SUBDIR: &'static str = crate::PLUGIN_ID;

    fn id(&self) -> &str {
        &self.setting
    }

    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

struct Claim(Setting);

impl HostMutation for Claim {
    type Snapshot = Record;

    fn owner(&self) -> &str {
        crate::PLUGIN_ID
    }

    fn id(&self) -> MutationId {
        self.0.key().to_string()
    }

    fn lifetime(&self) -> Lifetime {
        Lifetime::PortableSession
    }

    fn capture(&self) -> Result<Record> {
        Ok(Record {
            schema_version: RECORD_SCHEMA_VERSION,
            setting: self.0.key().to_string(),
            baseline: platform::capture(self.0)?,
        })
    }

    fn restore(record: Record) -> Result<()> {
        let setting = Setting::parse(&record.setting)
            .with_context(|| format!("`{}` is not an input setting", record.setting))?;
        platform::restore(setting, record.baseline)
    }
}

pub(super) fn claim(setting: Setting) -> Result<()> {
    match store().and_then(|store| store.claim(&Claim(platform::owner(setting)))) {
        Ok(()) => Ok(()),
        Err(error) if residency().is_resident() => {
            log::warn!(
                "the {} before this change was not saved: {error:#}",
                setting.name()
            );
            Ok(())
        }
        Err(error) => Err(error.context(format!(
            "the {} is unchanged, because its current value could not be saved to put back when qol exits",
            setting.name()
        ))),
    }
}

pub fn recover() {
    if let Ok(store) = store() {
        report(
            "after an unclean exit",
            &store.recover::<Claim>(crate::PLUGIN_ID, residency()),
        );
    }
}

pub fn release() {
    if let Ok(store) = store() {
        report(
            "at exit",
            &store.release_session::<Claim>(crate::PLUGIN_ID, residency()),
        );
    }
}

fn store() -> Result<SessionStore> {
    qol_config::data_subdir(LEDGER)
        .map(SessionStore::new)
        .context("there is no qol data directory to save it in")
}

fn residency() -> Residency {
    if HostResidency::current().is_resident() {
        Residency::Resident
    } else {
        Residency::Portable
    }
}

fn report(moment: &str, report: &RestoreReport) {
    if report.restored + report.failed + report.unreadable > 0 {
        log::info!(
            "put back {} input settings {moment}; {} failed, {} unreadable",
            report.restored,
            report.failed,
            report.unreadable
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_names_its_setting_and_keeps_the_baseline_it_saved() {
        let record = Record {
            schema_version: RECORD_SCHEMA_VERSION,
            setting: Setting::ALL[0].key().to_string(),
            baseline: Baseline::Numbers(vec![6, 10, 1]),
        };
        let text = serde_json::to_string(&record).unwrap();
        assert_eq!(serde_json::from_str::<Record>(&text).unwrap(), record);
        assert_eq!(record.id(), "pointer_speed");
        for setting in Setting::ALL {
            assert_eq!(Claim(setting).id(), setting.key());
            assert_eq!(Claim(setting).lifetime(), Lifetime::PortableSession);
        }
    }

    #[test]
    fn a_record_for_an_unknown_setting_is_refused_before_the_host_is_touched() {
        let error = Claim::restore(Record {
            schema_version: RECORD_SCHEMA_VERSION,
            setting: "volume".to_string(),
            baseline: Baseline::Number(1),
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("volume"), "{error}");
    }
}
