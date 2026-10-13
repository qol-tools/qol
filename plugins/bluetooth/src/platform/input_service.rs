use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const JOURNAL_FILE: &str = "disabled-input-services.json";
const PERIPHERAL_MAJOR_CLASS: u32 = 0x05;

pub(super) struct Journal {
    path: PathBuf,
}

impl Journal {
    pub(super) fn in_data_dir(data_dir: &Path) -> Self {
        Self::at(
            data_dir
                .join("plugins")
                .join(crate::PLUGIN_ID)
                .join(JOURNAL_FILE),
        )
    }

    pub(super) fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub(super) fn addresses(&self) -> Result<Vec<String>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("{} is not a Bluetooth journal", self.path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => {
                Err(error).with_context(|| format!("could not read {}", self.path.display()))
            }
        }
    }

    pub(super) fn contains(&self, address: &str) -> Result<bool> {
        Ok(self.addresses()?.iter().any(|entry| entry == address))
    }

    fn record(&self, address: &str) -> Result<()> {
        let mut addresses = self.addresses()?;
        if !addresses.iter().any(|entry| entry == address) {
            addresses.push(address.to_string());
        }
        self.write(&addresses)
    }

    fn clear(&self, address: &str) -> Result<()> {
        let mut addresses = self.addresses()?;
        addresses.retain(|entry| entry != address);
        self.write(&addresses)
    }

    fn write(&self, addresses: &[String]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        let content = serde_json::to_vec_pretty(addresses)
            .context("could not encode the Bluetooth journal")?;
        qol_fs::atomic_write_durable(&self.path, &content)
            .with_context(|| format!("could not write {}", self.path.display()))
    }
}

pub(super) fn restartable(class: u32, input_enabled: bool, journaled: bool) -> bool {
    journaled || input_enabled || (class >> 8) & 0x1f == PERIPHERAL_MAJOR_CLASS
}

pub(super) struct Cycle<E> {
    pub(super) disabled: Option<std::result::Result<(), E>>,
    pub(super) enabled: std::result::Result<(), E>,
}

pub(super) fn restart<E>(
    journal: &Journal,
    address: &str,
    journaled: bool,
    disable: impl FnOnce() -> std::result::Result<(), E>,
    enable: impl FnOnce() -> std::result::Result<(), E>,
) -> Result<Cycle<E>> {
    let disabled = if journaled {
        None
    } else {
        journal.record(address)?;
        Some(disable())
    };
    let enabled = enable();
    if enabled.is_ok() {
        forget(journal, address);
    }
    Ok(Cycle { disabled, enabled })
}

pub(super) fn restore(
    journal: &Journal,
    paired: impl Fn(&str) -> bool,
    mut enable: impl FnMut(&str) -> Result<()>,
) -> Vec<String> {
    let addresses = match journal.addresses() {
        Ok(addresses) => addresses,
        Err(error) => return vec![format!("{error:#}")],
    };
    let mut failures = Vec::new();
    for address in addresses {
        if !paired(&address) {
            forget(journal, &address);
            continue;
        }
        match enable(&address) {
            Ok(()) => forget(journal, &address),
            Err(error) => failures.push(format!(
                "qol turned off the input service of {address} and Windows could not turn it back on ({error:#}); tick its keyboard and mouse driver in Devices and Printers"
            )),
        }
    }
    failures
}

fn forget(journal: &Journal, address: &str) {
    if let Err(error) = journal.clear(address) {
        log::warn!("Bluetooth input service journal kept {address}: {error:#}");
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use anyhow::bail;

    use super::*;

    const MOUSE: &str = "AA:BB:CC:DD:EE:01";
    const KEYBOARD: &str = "AA:BB:CC:DD:EE:02";

    fn journal(dir: &tempfile::TempDir) -> Journal {
        Journal::in_data_dir(dir.path())
    }

    #[test]
    fn the_journal_lives_in_the_plugin_data_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = journal(&dir);
        assert_eq!(
            journal.path,
            dir.path()
                .join("plugins")
                .join(crate::PLUGIN_ID)
                .join(JOURNAL_FILE)
        );
        assert!(journal.addresses().unwrap().is_empty());
    }

    #[test]
    fn a_service_is_restartable_when_enabled_journaled_or_a_peripheral() {
        let mouse_class = 0x002580;
        let headset_class = 0x240404;
        let cases = [
            (headset_class, false, false, false),
            (headset_class, true, false, true),
            (headset_class, false, true, true),
            (mouse_class, false, false, true),
            (0, false, false, false),
        ];
        for (class, enabled, journaled, expected) in cases {
            assert_eq!(
                restartable(class, enabled, journaled),
                expected,
                "{class:06x} {enabled} {journaled}"
            );
        }
    }

    #[test]
    fn a_restart_journals_before_disabling_and_clears_after_enabling() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = journal(&dir);
        let seen = RefCell::new(Vec::new());
        let cycle = restart::<u32>(
            &journal,
            MOUSE,
            false,
            || {
                seen.borrow_mut().push(journal.contains(MOUSE).unwrap());
                Ok(())
            },
            || {
                seen.borrow_mut().push(journal.contains(MOUSE).unwrap());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*seen.borrow(), [true, true]);
        assert_eq!(cycle.disabled, Some(Ok(())));
        assert_eq!(cycle.enabled, Ok(()));
        assert!(!journal.contains(MOUSE).unwrap());
    }

    #[test]
    fn a_failed_enable_keeps_the_journal_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = journal(&dir);
        let cycle = restart(&journal, MOUSE, false, || Ok(()), || Err(87u32)).unwrap();
        assert_eq!(cycle.enabled, Err(87));
        assert!(journal.contains(MOUSE).unwrap());
    }

    #[test]
    fn a_journaled_service_is_enabled_without_disabling_it_again() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = journal(&dir);
        journal.record(MOUSE).unwrap();
        let disabled = RefCell::new(false);
        let cycle = restart::<u32>(
            &journal,
            MOUSE,
            true,
            || {
                *disabled.borrow_mut() = true;
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
        assert!(!*disabled.borrow());
        assert!(cycle.disabled.is_none());
        assert!(!journal.contains(MOUSE).unwrap());
    }

    #[test]
    fn an_unwritable_journal_never_disables_the_service() {
        let dir = tempfile::tempdir().expect("tempdir");
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"file").unwrap();
        let journal = Journal::at(blocker.join("journal.json"));
        let disabled = RefCell::new(false);
        let outcome = restart::<u32>(
            &journal,
            MOUSE,
            false,
            || {
                *disabled.borrow_mut() = true;
                Ok(())
            },
            || Ok(()),
        );
        assert!(outcome.is_err());
        assert!(!*disabled.borrow());
    }

    #[test]
    fn restore_enables_every_entry_and_keeps_only_the_failures() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = journal(&dir);
        journal.record(MOUSE).unwrap();
        journal.record(KEYBOARD).unwrap();
        let enabled = RefCell::new(Vec::new());
        let failures = restore(
            &journal,
            |_| true,
            |address| {
                enabled.borrow_mut().push(address.to_string());
                if address == KEYBOARD {
                    bail!("error 5");
                }
                Ok(())
            },
        );
        assert_eq!(*enabled.borrow(), [MOUSE, KEYBOARD]);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains(KEYBOARD), "{}", failures[0]);
        assert!(failures[0].contains("error 5"), "{}", failures[0]);
        assert_eq!(journal.addresses().unwrap(), [KEYBOARD]);
    }

    #[test]
    fn restore_drops_a_device_that_is_no_longer_paired_without_enabling_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = journal(&dir);
        journal.record(MOUSE).unwrap();
        journal.record(KEYBOARD).unwrap();
        let enabled = RefCell::new(Vec::new());
        let failures = restore(
            &journal,
            |address| address == MOUSE,
            |address| {
                enabled.borrow_mut().push(address.to_string());
                bail!("error 1168")
            },
        );
        assert_eq!(*enabled.borrow(), [MOUSE]);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains(MOUSE), "{}", failures[0]);
        assert_eq!(journal.addresses().unwrap(), [MOUSE]);
    }

    #[test]
    fn restore_reports_an_unreadable_journal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = journal(&dir);
        std::fs::create_dir_all(journal.path.parent().unwrap()).unwrap();
        std::fs::write(&journal.path, b"not json").unwrap();
        let failures = restore(&journal, |_| true, |_| Ok(()));
        assert_eq!(failures.len(), 1);
        assert!(
            failures[0].contains("not a Bluetooth journal"),
            "{}",
            failures[0]
        );
    }
}
