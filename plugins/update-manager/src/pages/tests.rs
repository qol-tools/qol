use chrono::NaiveDateTime;
use qol_config::pages::{Control, Enter, Go, Notice, Page, Pages};

use super::{build, press, ROOT};
use crate::session::{Act, Helper, InUse, Journal, Session, Task, MINT_DEFAULT};
use crate::updates::{
    Auto, Failure, Finish, Held, Hours, Mint, Package, Reason, Refusal, Run, RunBy, Unit, Update,
    Updates,
};

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").expect("time")
}

fn now() -> NaiveDateTime {
    at("2026-10-14 14:00")
}

fn update(id: &str, label: &str, count: usize, unit: Unit) -> Update {
    Update {
        id: id.into(),
        tag: "Update Manager".into(),
        label: label.into(),
        name: label.into(),
        why: "A security fix. Restarts nothing.".into(),
        restart: false,
        held: None,
        unit,
        packages: (0..count)
            .map(|index| Package {
                name: format!("{id}-{index}"),
                detail: "A package".into(),
                bytes: Some(95_900),
                target: format!("{id}-{index}=1"),
            })
            .collect(),
        running: None,
        finish: String::new(),
        stops_sound: false,
        caution: None,
        unattended: true,
    }
}

fn mint() -> Session {
    let mut nvidia = update("apt:nvidia", "NVIDIA driver 580.178", 20, Unit::Packages);
    nvidia.restart = true;
    nvidia.running = Some("580.173".into());
    nvidia.caution = Some((
        "Games opened before the restart may fail".into(),
        "The new NVIDIA driver loads at the restart; open apps keep working".into(),
    ));
    let mut apparmor = update("apt:apparmor", "AppArmor 4.0.1", 3, Unit::Packages);
    apparmor.held = Some(Held::Phasing(10));
    let mut gl = update("flatpak:gl-nvidia", "NVIDIA GL runtimes", 8, Unit::Runtimes);
    gl.tag = "Flatpak".into();
    gl.unattended = false;
    let mut dbx = update("fwupd:dbx", "UEFI Secure Boot list", 1, Unit::Firmware);
    dbx.tag = "Firmware".into();
    dbx.name = "the Secure Boot list".into();
    dbx.restart = true;
    dbx.unattended = false;
    let mut session = Session::new(Journal::default(), false);
    session.install_log =
        Some("/home/user/.local/share/qol-tray/plugins/qol-update-manager/install.log".into());
    session.apply_updates(
        Updates {
            list: vec![
                nvidia,
                update("apt:urllib3", "urllib3 2.0.7", 1, Unit::Packages),
                apparmor,
                gl,
                dbx,
            ],
            problems: Vec::new(),
        },
        at("2026-10-14 13:58"),
    );
    session.host.admin = true;
    session.host.log = Some("/var/log/apt/history.log".into());
    session.host.finish = vec![Finish {
        id: "kernel".into(),
        label: "Linux kernel 6.8.0-147".into(),
        name: "Linux kernel 6.8.0-147".into(),
        finish: "Installed today at 07:45. Until a restart, 6.8.0-146 keeps running.".into(),
        short: "6.8.0-146 runs until a restart".into(),
    }];
    session.host.runs = vec![Run {
        start: at("2026-10-14 07:45"),
        end: Some(at("2026-10-14 08:02")),
        by: RunBy::Automatic,
        upgraded: 17,
        groups: vec![("Bluetooth".into(), 5), ("kernel".into(), 8)],
        added: 13,
        added_detail: "Linux kernel 6.8.0-147 itself, and 12 more".into(),
        after_startup: Some(45),
        bluetooth: Some(at("2026-10-14 07:46")),
    }];
    session.host.mint = Some(Mint {
        hours: Some(MINT_DEFAULT),
        startup_delay: 60,
        ours: false,
        next: Some(at("2026-10-15 00:37")),
        last: Some(at("2026-10-14 07:45")),
        auto: Auto {
            packages: true,
            spices: true,
            flatpak: false,
        },
    });
    session
}

fn page<'a>(pages: &'a Pages, id: &str) -> &'a Page {
    pages
        .page(id)
        .unwrap_or_else(|| panic!("page `{id}` is missing"))
}

fn label(pages: &Pages, page_id: &str, row: &str) -> String {
    page(pages, page_id)
        .row(row)
        .unwrap_or_else(|| panic!("row `{row}` is missing on `{page_id}`"))
        .label
        .clone()
}

fn detail(pages: &Pages, page_id: &str, row: &str) -> String {
    page(pages, page_id)
        .row(row)
        .and_then(|row| row.detail.clone())
        .unwrap_or_default()
}

fn broken(pages: &Pages) -> Vec<String> {
    pages
        .problems()
        .into_iter()
        .chain(pages.dead_ends())
        .collect()
}

fn crowded() -> Session {
    let long = "x".repeat(48);
    let list = (0..400)
        .map(|index| {
            let mut one = update(
                &format!("apt:{index:03}-{long}"),
                &format!("Package {index:03} {long}"),
                if index == 7 { 150 } else { 3 },
                Unit::Packages,
            );
            one.restart = index % 10 == 0;
            one.running = one.restart.then(|| "6.8.0-146".into());
            one.held = (index % 25 == 1).then_some(Held::Phasing(10));
            one.caution = (index % 50 == 0).then(|| {
                (
                    format!("Caution {index} {long}"),
                    format!("What changes at the restart {long}"),
                )
            });
            one
        })
        .collect();
    let mut session = Session::new(Journal::default(), false);
    session.apply_updates(
        Updates {
            list,
            problems: Vec::new(),
        },
        at("2026-10-14 13:58"),
    );
    session.failed = Some(crate::session::Failed {
        updates: session.list().to_vec(),
        failure: Failure {
            done: 0,
            reason: Reason::Locked(None),
        },
    });
    session
}

fn deepest(board: &super::Board) -> usize {
    let mut edges: std::collections::HashMap<&str, Vec<(&str, usize)>> =
        std::collections::HashMap::new();
    for (id, page) in &board.pages.pages {
        for row in page.rows() {
            match &row.enter {
                Some(Enter::Open(target)) => edges.entry(id).or_default().push((target, 1)),
                Some(Enter::Replace(target)) => edges.entry(id).or_default().push((target, 0)),
                _ => {}
            }
        }
    }
    for ((id, _), (_, go)) in &board.acts {
        match go {
            Some(Go::Open(target)) => edges.entry(id).or_default().push((target, 1)),
            Some(Go::Replace(target)) => edges.entry(id).or_default().push((target, 0)),
            _ => {}
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut todo = vec![(board.pages.root.as_str(), 1)];
    let mut deepest = 1;
    while let Some((id, level)) = todo.pop() {
        if level > 8 || !seen.insert((id, level)) {
            continue;
        }
        deepest = deepest.max(level);
        for (target, step) in edges.get(id).into_iter().flatten() {
            todo.push((target, level + step));
        }
    }
    deepest
}

#[test]
fn every_answer_fits_the_message_limit_with_hundreds_of_updates() {
    let board = build(&crowded(), now());
    assert_eq!(broken(&board.pages), Vec::<String>::new());
    let largest = board
        .pages
        .pages
        .values()
        .map(|page| serde_json::to_vec(page).expect("page").len())
        .max()
        .unwrap_or(0);
    let deepest = deepest(&board);
    assert!(deepest < 8, "pages open each other deeper and deeper");
    assert!(
        (deepest + 1) * largest + 1024 < 64 * 1024,
        "{deepest} open pages of up to {largest} bytes do not fit"
    );
    let first = page(&board.pages, "waiting");
    assert_eq!(
        first
            .rows()
            .filter(|row| row.id.starts_with("update:"))
            .count(),
        40
    );
    assert_eq!(label(&board.pages, "waiting", "more"), "Show 41 to 80");
    assert_eq!(label(&board.pages, "waiting~10", "more"), "Show 1 to 40");
    let big = format!("update:apt:007-{}", "x".repeat(48));
    assert_eq!(label(&board.pages, &big, "more"), "Show 41 to 80");
    assert_eq!(
        label(&board.pages, &format!("{big}~4"), "more"),
        "Show 1 to 40"
    );
}

#[test]
fn every_page_is_consistent_in_every_state() {
    let mut session = mint();
    assert_eq!(broken(&build(&session, now()).pages), Vec::<String>::new());
    session.borrowed = true;
    session.ask = Some(crate::session::Proposal::Tonight);
    session.in_use = Some(InUse {
        label: "Luna 2".into(),
        bluetooth: true,
    });
    session.host.restart_at = Some(at("2026-10-15 01:30"));
    assert_eq!(broken(&build(&session, now()).pages), Vec::<String>::new());
    session.host.admin = false;
    session.failed = Some(crate::session::Failed {
        updates: vec![session.list()[0].clone()],
        failure: Failure {
            done: 0,
            reason: Reason::Locked(Some("Mint's Update Manager".into())),
        },
    });
    let pages = build(&session, now()).pages;
    assert_eq!(broken(&pages), Vec::<String>::new());
    assert!(pages.page("hours").is_none());
    let hours = page(&pages, ROOT).row("hours").expect("hours row");
    assert_eq!(hours.tag, None);
    assert_eq!(hours.detail, None);
    assert!(page(&pages, ROOT).row("locked").is_some());
    assert!(page(&pages, ROOT).row("borrowed").is_none());
    assert!(page(&pages, "changes").row("guard").is_none());
    assert!(page(&pages, "changes").row("hold").is_none());
}

#[test]
fn the_first_page_reads_like_the_design() {
    let pages = build(&mint(), now()).pages;
    assert_eq!(pages.root, ROOT);
    assert_eq!(page(&pages, ROOT).title, "Update Manager");
    assert_eq!(label(&pages, ROOT, "waiting"), "33 updates waiting");
    assert_eq!(
        detail(&pages, ROOT, "waiting"),
        "30 can install, 3 held by Ubuntu"
    );
    assert_eq!(label(&pages, ROOT, "restart"), "Linux kernel 6.8.0-147");
    assert_eq!(
        detail(&pages, ROOT, "restart"),
        "6.8.0-146 runs until a restart"
    );
    assert_eq!(
        page(&pages, ROOT)
            .row("auto")
            .and_then(|row| row.value.clone())
            .as_deref(),
        Some("packages and spices")
    );
    assert_eq!(
        page(&pages, ROOT)
            .row("hours")
            .and_then(|row| row.value.clone())
            .as_deref(),
        Some("00:00 to 01:00")
    );
    assert_eq!(label(&pages, ROOT, "last"), "Today, 07:45");
    assert_eq!(
        detail(&pages, ROOT, "last"),
        "Mint upgraded 17 packages. Bluetooth restarted at 07:46."
    );
    assert_eq!(detail(&pages, ROOT, "changes"), "Nothing so far");
    assert_eq!(
        page(&pages, ROOT)
            .row("guard")
            .and_then(|row| row.control.clone()),
        Some(Control::Toggle(false))
    );
}

#[test]
fn the_waiting_page_puts_restarts_first_and_held_last() {
    let pages = build(&mint(), now()).pages;
    let waiting = page(&pages, "waiting");
    let ids: Vec<&str> = waiting.rows().map(|row| row.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "install",
            "update:apt:nvidia",
            "update:fwupd:dbx",
            "update:apt:urllib3",
            "update:flatpak:gl-nvidia",
            "update:apt:apparmor",
        ]
    );
    assert_eq!(label(&pages, "waiting", "install"), "30 updates");
    assert_eq!(
        detail(&pages, "waiting", "install"),
        "Installing asks for your password."
    );
    let mut locked = mint();
    locked.host.admin = false;
    assert_eq!(
        detail(&build(&locked, now()).pages, "waiting", "install"),
        "Installing asks for an administrator's password."
    );
    assert_eq!(
        waiting.row("install").and_then(|row| row.control.clone()),
        Some(Control::Button("Install all".into()))
    );
    assert_eq!(
        page(&pages, "waiting").groups[1].colophon.as_deref(),
        Some("each tagged with its updater.")
    );
    assert!(waiting
        .row("update:apt:nvidia")
        .is_some_and(|row| row.tag.as_deref() == Some("Update Manager")));
    assert!(waiting
        .row("update:apt:apparmor")
        .is_some_and(|row| row.dim));
    assert_eq!(
        detail(&pages, "waiting", "update:apt:nvidia"),
        "20 packages, needs a restart"
    );
    let one = page(&pages, "update:apt:nvidia");
    assert_eq!(one.title, "nvidia driver 580.178");
    assert_eq!(
        one.rows()
            .filter(|row| row.id.starts_with("package:"))
            .count(),
        20
    );
}

#[test]
fn one_shared_updater_is_said_once() {
    let mut session = mint();
    let list: Vec<Update> = session
        .list()
        .iter()
        .cloned()
        .map(|mut update| {
            update.tag = "Update Manager".into();
            update
        })
        .collect();
    session.apply_updates(
        Updates {
            list,
            problems: Vec::new(),
        },
        at("2026-10-14 13:58"),
    );
    let pages = build(&session, now()).pages;
    let waiting = page(&pages, "waiting");
    assert_eq!(
        waiting.groups[1].colophon.as_deref(),
        Some("all from update manager.")
    );
    assert!(waiting
        .rows()
        .filter(|row| row.id.starts_with("update:"))
        .all(|row| row.tag.is_none()));
}

#[test]
fn install_asks_first_then_starts_one_job() {
    let mut session = mint();
    let pages = build(&session, now()).pages;
    let all = super::updates::install_all(&session);
    let install = page(&pages, &all);
    assert_eq!(install.groups[0].title, "install 30 updates now?");
    assert_eq!(
        label(&pages, &all, "restart"),
        "NVIDIA driver 580.178 and the Secure Boot list"
    );
    assert_eq!(label(&pages, &all, "held"), "AppArmor 4.0.1 stays held");
    assert_eq!(
        install.row("cancel").and_then(|row| row.enter.clone()),
        Some(Enter::Back)
    );
    let outcome = press(&mut session, &all, "go", now());
    assert_eq!(outcome.go, Some(Go::Back));
    let Some(Task::Install(list)) = outcome.task else {
        panic!("install must start a job");
    };
    assert_eq!(list.len(), 4);
    let pages = build(&session, now()).pages;
    assert_eq!(label(&pages, ROOT, "waiting"), "Waiting for the password");
    assert_eq!(detail(&pages, ROOT, "waiting"), "To install 30 updates");
    assert_eq!(
        detail(&pages, "waiting", "update:apt:nvidia"),
        "waits for the password"
    );
    session.progress(0);
    let pages = build(&session, now()).pages;
    assert_eq!(label(&pages, ROOT, "waiting"), "Installing 30 updates");
    assert!(pages.page(&all).is_none());
    assert_eq!(
        detail(&pages, "waiting", "update:apt:nvidia"),
        "installing, 0 of 20 packages"
    );
    let again = press(&mut session, "waiting", "install", now());
    assert!(matches!(again.notice, Some(Notice::Failure(_))));
}

#[test]
fn an_administrator_hold_installs_like_any_update() {
    let mut session = mint();
    let held = session
        .updates
        .as_mut()
        .and_then(|updates| {
            updates
                .list
                .iter_mut()
                .find(|update| update.id == "apt:nvidia")
        })
        .expect("nvidia");
    held.held = Some(Held::Administrator);
    held.why = "An administrator holds it at 580.173".into();
    let pages = build(&session, now()).pages;
    assert_eq!(
        detail(&pages, ROOT, "waiting"),
        "30 can install, 3 held by Ubuntu"
    );
    let row = page(&pages, "waiting")
        .row("update:apt:nvidia")
        .expect("nvidia row");
    assert!(!row.dim);
    let all = super::updates::install_all(&session);
    assert_eq!(
        label(&pages, &all, "release:apt:nvidia"),
        "Its hold comes off"
    );
    assert_eq!(label(&pages, &all, "held"), "AppArmor 4.0.1 stays held");
    assert!(pages.page("install:apt:apparmor").is_none());
    let outcome = press(&mut session, &all, "go", now());
    let Some(Task::Install(list)) = outcome.task else {
        panic!("install all must take the held driver too");
    };
    assert!(list
        .iter()
        .any(|update| update.held == Some(Held::Administrator)));
}

#[test]
fn a_failed_install_offers_a_retry_and_the_log() {
    let mut session = mint();
    press(&mut session, "install:apt:nvidia", "go", now());
    session.installed(Err(Failure {
        done: 0,
        reason: Reason::Locked(Some("Mint's Update Manager".into())),
    }));
    let pages = build(&session, now()).pages;
    assert_eq!(
        label(&pages, ROOT, "failed"),
        "NVIDIA driver 580.178 did not install"
    );
    assert_eq!(
        detail(&pages, ROOT, "failed"),
        "Another installer was already running. Nothing changed; it still waits."
    );
    assert_eq!(
        detail(&pages, "failed", "reason"),
        "Mint's Update Manager was installing at the same time"
    );
    assert_eq!(
        label(&pages, "failed", "running:apt:nvidia"),
        "580.173 keeps running"
    );
    let retry = press(&mut session, "failed", "retry", now());
    assert!(matches!(retry.task, Some(Task::Install(ref list)) if list.len() == 1));
    assert!(session.failed.is_none());
}

#[test]
fn the_restart_page_sets_the_time_and_the_switch() {
    let mut session = mint();
    let pages = build(&session, now()).pages;
    assert_eq!(
        page(&pages, "restart").groups[0].colophon.as_deref(),
        Some("linux kernel 6.8.0-147 finishes when the computer restarts.")
    );
    assert_eq!(
        label(&pages, "restart", "tonight"),
        "Restart tonight at 01:30"
    );
    assert_eq!(
        detail(&pages, "restart", "tonight"),
        "After Mint's run at 00:37 ends. Stays set if qol stops."
    );
    assert_eq!(page(&pages, "time").focus.as_deref(), Some("time:01:30"));
    let outcome = press(&mut session, "time", "time:03:00", now());
    assert_eq!(outcome.go, Some(Go::Back));
    assert!(outcome.task.is_none());
    let outcome = press(&mut session, "restart", "tonight", now());
    assert_eq!(
        outcome.task,
        Some(Task::Restart(Some(at("2026-10-15 03:00"))))
    );
    let busy = press(&mut session, "restart", "tonight", now());
    assert!(matches!(busy.notice, Some(Notice::Failure(_))));
}

#[test]
fn a_borrowed_computer_asks_before_changing_the_owners_settings() {
    let mut session = mint();
    session.borrowed = true;
    let outcome = press(&mut session, "restart", "tonight", now());
    assert_eq!(outcome.go, Some(Go::Open("ask".into())));
    assert!(outcome.task.is_none());
    let pages = build(&session, now()).pages;
    assert_eq!(
        page(&pages, "ask").groups[0].title,
        "restart the owner's computer tonight?"
    );
    let set = press(&mut session, "ask", "set", now());
    assert_eq!(set.go, Some(Go::Back));
    assert!(matches!(set.task, Some(Task::Restart(Some(_)))));
    assert!(session.ask.is_none());

    let picked = press(&mut session, "from", "from:02:00", now());
    assert_eq!(picked.go, Some(Go::Replace("ask".into())));
    let pages = build(&session, now()).pages;
    assert_eq!(
        page(&pages, "ask")
            .row("after")
            .and_then(|row| row.value.clone())
            .as_deref(),
        Some("02:00 to 03:00")
    );
    let held = press(&mut session, "ask", "hold", now());
    assert_eq!(held.task, Some(Task::Rules));
    assert!(session.rules.hold);
}

#[test]
fn hours_and_switches_change_through_tasks() {
    let mut session = mint();
    let outcome = press(&mut session, "until", "until:03:00", now());
    assert_eq!(outcome.go, Some(Go::Back));
    let Some(Task::Hours { hours, restart, .. }) = outcome.task else {
        panic!("a new until must write the hours");
    };
    assert_eq!(
        hours,
        Some(Hours {
            until: crate::updates::Clock::hm(3, 0),
            ..MINT_DEFAULT
        })
    );
    assert_eq!(restart, None);
    let switch = press(&mut session, "auto", "flatpak", now());
    assert!(matches!(
        switch.task,
        Some(Task::Auto {
            on: true,
            was: false,
            ..
        })
    ));
    session.done(&switch.task.expect("task"), Err(Refusal::Denied), now());
    let pages = build(&session, now()).pages;
    assert_eq!(
        detail(&pages, "auto", "flatpak"),
        "the password was not accepted"
    );
}

#[test]
fn rules_and_changes_list_what_qol_holds() {
    let mut session = mint();
    let outcome = press(&mut session, ROOT, "guard", now());
    assert_eq!(outcome.task, Some(Task::Rules));
    session.rules.helper = Helper::Ready;
    session.host.restart_at = Some(at("2026-10-15 01:30"));
    let pages = build(&session, now()).pages;
    assert_eq!(detail(&pages, ROOT, "changes"), "Restart tonight");
    assert_eq!(label(&pages, "changes", "restart"), "Restart tonight");
    let cancel = press(&mut session, "changes", "restart", now());
    assert_eq!(cancel.task, Some(Task::Restart(None)));
    assert!(matches!(
        session.apply(&Act::Guard, now()),
        Ok(Some(Task::Rules))
    ));
    assert!(!session.rules.guard);
}

#[test]
fn a_hold_the_password_prompt_refused_is_not_listed_as_a_change() {
    let mut session = mint();
    let held = press(&mut session, ROOT, "hold", now());
    assert_eq!(held.task, Some(Task::Rules));
    session.rules.helper = Helper::Starting;
    let pages = build(&session, now()).pages;
    assert_eq!(detail(&pages, ROOT, "changes"), "Nothing so far");
    assert_eq!(
        detail(&pages, ROOT, "hold"),
        "Waiting for the password, to hold Mint's run"
    );
    session.rules.helper = Helper::Failed(Refusal::Dismissed);
    let pages = build(&session, now()).pages;
    assert_eq!(detail(&pages, ROOT, "changes"), "Nothing so far");
    assert_eq!(
        detail(&pages, ROOT, "hold"),
        "The password prompt was closed."
    );
}

#[test]
fn rules_the_password_prompt_refused_switch_off_and_retry() {
    let mut session = mint();
    press(&mut session, ROOT, "guard", now());
    press(&mut session, ROOT, "hold", now());
    session.rules.helper = Helper::Failed(Refusal::TimedOut);
    let pages = build(&session, now()).pages;
    assert_eq!(
        detail(&pages, ROOT, "guard"),
        "The password prompt timed out."
    );
    assert_eq!(
        page(&pages, ROOT)
            .row("hold")
            .and_then(|row| row.control.clone()),
        Some(Control::Toggle(true))
    );
    assert_eq!(label(&pages, ROOT, "retry"), "Try again");
    assert_eq!(
        press(&mut session, ROOT, "hold", now()).task,
        Some(Task::Rules)
    );
    assert!(!session.rules.hold);
    assert!(!session.want_helper());
    let pages = build(&session, now()).pages;
    assert_eq!(
        detail(&pages, ROOT, "hold"),
        "Nothing installs until you switch it off"
    );
    assert!(page(&pages, ROOT).row("retry").is_some());
    press(&mut session, ROOT, "retry", now());
    assert_eq!(session.rules.helper, Helper::Off);
    assert!(session.rules.guard);
    assert!(session.want_helper());
    session.rules.helper = Helper::Failed(Refusal::Dismissed);
    press(&mut session, ROOT, "guard", now());
    assert!(!session.rules.guard);
    assert_eq!(session.rules.helper, Helper::Off);
    assert!(!session.want_helper());
    let pages = build(&session, now()).pages;
    assert!(page(&pages, ROOT).row("retry").is_none());
    assert_eq!(
        page(&pages, ROOT)
            .row("guard")
            .and_then(|row| row.control.clone()),
        Some(Control::Toggle(false))
    );
}

#[test]
fn unknown_rows_answer_with_a_notice() {
    let mut session = mint();
    let outcome = press(&mut session, ROOT, "nowhere", now());
    assert!(matches!(outcome.notice, Some(Notice::Failure(_))));
    assert!(outcome.task.is_none());
}
