use anyhow::{bail, Context, Result};

use super::session::Session;
use crate::claims::Ledger;
use crate::handler::backends::{desktop_entry, mimeapps};
use crate::handler::{self, Claim, HandlerStore};
use crate::host::OpenerSpec;
use crate::report::{Lifetime, Owner, Report, Restore, State};
use crate::role::Role;
use crate::xdg::XdgDirs;

const DIRECTORY_MIME: &str = "inode/directory";
const REVEAL_REASON: &str = "The desktop answers it.";
const DESKTOP_FALLBACK_NAME: &str = "the desktop";

pub(super) fn probe_open(session: &Session) -> Report {
    match XdgDirs::from_env() {
        Some(dirs) => probe_open_in(&dirs, &session.desktops()),
        None => Report {
            state: State::Unknown {
                reason: "the home folder could not be read".to_string(),
            },
            lifetimes: Vec::new(),
            ..open_report(Owner::Unreadable, Vec::new())
        },
    }
}

pub(super) fn probe_reveal() -> Report {
    Report::impossible(Role::FolderReveal, REVEAL_REASON)
}

pub(super) fn probe_open_in(dirs: &XdgDirs, desktops: &[String]) -> Report {
    let texts = mimeapps::cascade_paths(dirs, desktops)
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect::<Vec<_>>();
    let default = mimeapps::effective_default(&texts, DIRECTORY_MIME, |id| {
        id != desktop_entry::OPENER_ID && desktop_entry::find_entry(dirs, id).is_some()
    });
    let name = default
        .as_deref()
        .and_then(|id| desktop_entry::entry_owner(dirs, id));
    let reveal_owner = name.as_deref().unwrap_or(DESKTOP_FALLBACK_NAME);
    let kept = vec![format!(
        "Show a file in its folder stays with {reveal_owner}."
    )];
    open_report(name.map_or(Owner::Unreadable, Owner::Named), kept)
}

fn open_report(owner: Owner, kept: Vec<String>) -> Report {
    Report {
        role: Role::FolderOpen,
        state: State::Silent,
        owner,
        triggers: vec!["opening folders".to_string()],
        kept,
        restore: Restore::Exact,
        lifetimes: vec![Lifetime::PortableSession, Lifetime::ResidentPolicy],
    }
}

pub(super) fn claim_open(
    session: &Session,
    role: Role,
    opener: &OpenerSpec,
    plugin_id: &str,
    lifetime: Lifetime,
    ledger: &Ledger,
) -> Result<()> {
    if role != Role::FolderOpen {
        bail!("{} is not a handler role on this system", role.id());
    }
    let dirs = XdgDirs::from_env().context("the home folder could not be read")?;
    allow(&probe_open_in(&dirs, &session.desktops()), lifetime)?;
    claim_open_in(&dirs, opener, plugin_id, lifetime, ledger)
}

fn allow(report: &Report, lifetime: Lifetime) -> Result<()> {
    match &report.state {
        State::Silent => {}
        State::NeedsConfirmation { ask } | State::NeedsPermission { ask } => {
            bail!("{} needs approval first: {ask}", report.role.id())
        }
        State::Impossible { reason } | State::Unknown { reason } => bail!("{reason}"),
    }
    if !report.lifetimes.contains(&lifetime) {
        bail!("{} cannot be kept for {lifetime:?}", report.role.id());
    }
    Ok(())
}

pub(super) fn claim_open_in(
    dirs: &XdgDirs,
    opener: &OpenerSpec,
    plugin_id: &str,
    lifetime: Lifetime,
    ledger: &Ledger,
) -> Result<()> {
    let entry_key = desktop_entry::opener_path(dirs)
        .to_string_lossy()
        .into_owned();
    let entry = desktop_entry::render_opener(&opener.exec);
    handler::claim_value(
        ledger,
        &Claim {
            role: Role::FolderOpen,
            plugin_id,
            lifetime,
            store: HandlerStore::DesktopEntry,
            key: &entry_key,
            applied: &entry,
        },
    )?;
    let list_key = mimeapps::key_for(&mimeapps::user_list_path(dirs), DIRECTORY_MIME);
    handler::claim_value(
        ledger,
        &Claim {
            role: Role::FolderOpen,
            plugin_id,
            lifetime,
            store: HandlerStore::Mimeapps,
            key: &list_key,
            applied: desktop_entry::OPENER_ID,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::ReleaseOutcome;
    use std::path::{Path, PathBuf};

    fn dirs(root: &Path) -> XdgDirs {
        XdgDirs {
            config_home: root.join("config"),
            config_dirs: Vec::new(),
            data_home: root.join("data"),
            data_dirs: vec![root.join("system")],
        }
    }

    fn install_system_entry(dirs: &XdgDirs, id: &str, name: &str) {
        let applications = dirs.data_dirs[0].join("applications");
        std::fs::create_dir_all(&applications).unwrap();
        std::fs::write(
            applications.join(id),
            format!("[Desktop Entry]\nName={name}\nMimeType=inode/directory;\n"),
        )
        .unwrap();
    }

    fn system_default(dirs: &XdgDirs, value: &str) {
        let applications = dirs.data_dirs[0].join("applications");
        std::fs::create_dir_all(&applications).unwrap();
        std::fs::write(
            applications.join("mimeapps.list"),
            format!("[Default Applications]\ninode/directory={value}\n"),
        )
        .unwrap();
    }

    fn opener() -> OpenerSpec {
        OpenerSpec {
            exec: PathBuf::from("/opt/qol/qol-tray"),
        }
    }

    #[test]
    fn the_owner_is_the_name_of_the_effective_default_entry() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        install_system_entry(&dirs, "nemo.desktop", "Files");
        system_default(&dirs, "nemo.desktop;thunar.desktop");
        let report = probe_open_in(&dirs, &[]);
        assert_eq!(report.role, Role::FolderOpen);
        assert_eq!(report.state, State::Silent);
        assert_eq!(report.owner, Owner::Named("Files".to_string()));
        assert_eq!(report.triggers, vec!["opening folders".to_string()]);
        assert_eq!(
            report.kept,
            vec!["Show a file in its folder stays with Files.".to_string()]
        );
        assert_eq!(report.restore, Restore::Exact);
        assert_eq!(
            report.lifetimes,
            vec![Lifetime::PortableSession, Lifetime::ResidentPolicy]
        );
    }

    #[test]
    fn the_owner_is_the_program_when_the_entry_name_is_generic() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        let applications = dirs.data_dirs[0].join("applications");
        std::fs::create_dir_all(&applications).unwrap();
        std::fs::write(
            applications.join("nemo.desktop"),
            "[Desktop Entry]\nName=Files\nExec=nemo %U\nMimeType=inode/directory;\n",
        )
        .unwrap();
        system_default(&dirs, "nemo.desktop");
        let report = probe_open_in(&dirs, &[]);
        assert_eq!(report.owner, Owner::Named("Nemo".to_string()));
        assert_eq!(
            report.kept,
            vec!["Show a file in its folder stays with Nemo.".to_string()]
        );
    }

    #[test]
    fn an_unreadable_default_falls_back_without_naming_a_tool() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        let report = probe_open_in(&dirs, &[]);
        assert_eq!(report.owner, Owner::Unreadable);
        assert_eq!(
            report.kept,
            vec!["Show a file in its folder stays with the desktop.".to_string()]
        );
    }

    #[test]
    fn a_desktop_specific_list_wins_over_the_plain_one() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        install_system_entry(&dirs, "nemo.desktop", "Files");
        install_system_entry(&dirs, "dolphin.desktop", "Dolphin");
        system_default(&dirs, "nemo.desktop");
        std::fs::create_dir_all(&dirs.config_home).unwrap();
        std::fs::write(
            dirs.config_home.join("x-cinnamon-mimeapps.list"),
            "[Default Applications]\ninode/directory=dolphin.desktop\n",
        )
        .unwrap();
        let report = probe_open_in(&dirs, &["x-cinnamon".to_string()]);
        assert_eq!(report.owner, Owner::Named("Dolphin".to_string()));
    }

    #[test]
    fn the_report_looks_through_qol_own_claim_to_the_real_file_manager() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        install_system_entry(&dirs, "nemo.desktop", "Files");
        system_default(&dirs, "nemo.desktop");
        let ledger = Ledger::at(root.path().join("claims.json"));
        claim_open_in(
            &dirs,
            &opener(),
            "explorer",
            Lifetime::PortableSession,
            &ledger,
        )
        .unwrap();
        let report = probe_open_in(&dirs, &[]);
        assert_eq!(report.owner, Owner::Named("Files".to_string()));
    }

    #[test]
    fn claiming_writes_the_opener_and_the_default_then_release_removes_both() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        let ledger = Ledger::at(root.path().join("claims.json"));
        let list = mimeapps::user_list_path(&dirs);
        std::fs::create_dir_all(&dirs.config_home).unwrap();
        let original = "[Default Applications]\ninode/directory=thunar.desktop\n";
        std::fs::write(&list, original).unwrap();

        claim_open_in(
            &dirs,
            &opener(),
            "explorer",
            Lifetime::PortableSession,
            &ledger,
        )
        .unwrap();

        let entry = std::fs::read_to_string(desktop_entry::opener_path(&dirs)).unwrap();
        assert!(entry.contains("Exec=/opt/qol/qol-tray defaults open folder.open %U\n"));
        assert!(entry.contains("NoDisplay=true\n"));
        assert!(std::fs::read_to_string(&list)
            .unwrap()
            .contains("inode/directory=qol-open.desktop\n"));
        let records = ledger.load().unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|record| record.plugin_id == "explorer"));
        assert_eq!(
            ledger.holder(Role::FolderOpen).unwrap().as_deref(),
            Some("explorer")
        );

        let outcome = handler::release_role(Role::FolderOpen, &ledger).unwrap();
        assert_eq!(outcome, ReleaseOutcome::Restored);
        assert!(!desktop_entry::opener_path(&dirs).exists());
        assert_eq!(std::fs::read_to_string(&list).unwrap(), original);
        assert!(ledger.load().unwrap().is_empty());
    }

    #[test]
    fn claim_refuses_a_role_that_is_not_a_handler() {
        let ledger = Ledger::at(std::env::temp_dir().join("qol-host-defaults-unused.json"));
        let session = Session::from_parts("X-Cinnamon", qol_platform::LinuxDisplayBackend::X11);
        for role in [Role::FolderReveal, Role::SlotLauncher] {
            let result = claim_open(
                &session,
                role,
                &opener(),
                "explorer",
                Lifetime::PortableSession,
                &ledger,
            );
            assert!(result.is_err(), "{role:?}");
        }
    }

    #[test]
    fn allow_follows_the_report_state_and_lifetimes() {
        let silent = open_report(Owner::Unreadable, Vec::new());
        let only_resident = Report {
            lifetimes: vec![Lifetime::ResidentPolicy],
            ..silent.clone()
        };
        let confirm = Report {
            state: State::NeedsConfirmation {
                ask: "Confirm.".to_string(),
            },
            ..silent.clone()
        };
        let impossible = Report::impossible(Role::FolderOpen, "no");
        let cases = [
            ("silent portable", &silent, Lifetime::PortableSession, true),
            ("silent resident", &silent, Lifetime::ResidentPolicy, true),
            (
                "resident only, portable asked",
                &only_resident,
                Lifetime::PortableSession,
                false,
            ),
            (
                "needs confirmation",
                &confirm,
                Lifetime::PortableSession,
                false,
            ),
            ("impossible", &impossible, Lifetime::PortableSession, false),
        ];
        for (label, report, lifetime, ok) in cases {
            assert_eq!(allow(report, lifetime).is_ok(), ok, "case: {label}");
        }
    }

    #[test]
    fn reveal_is_impossible_with_the_reason() {
        let report = probe_reveal();
        assert_eq!(report.role, Role::FolderReveal);
        assert_eq!(
            report.state,
            State::Impossible {
                reason: "The desktop answers it.".to_string()
            }
        );
    }
}
