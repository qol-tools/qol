use chrono::NaiveDateTime;
use qol_config::pages::{Control, Enter, Go, Notice, Page, Pages};

use super::{build, press, ROOT};
use crate::cleanup::fake::{
    self, found, ALONG, BY_HAND, EXTRA, LAST, LISTED, MINT_BY_HAND, MINT_KERNELS, OLD, ORPHANS,
    SCRIPTED, UNATTENDED_KERNELS,
};
use crate::cleanup::{Amount, Consent, Found, Live, Size, State, Why};
use crate::platform::Refusal;
use crate::session::{Helper, InUse, Journal, Op, Session, Task, MINT_DEFAULT};
use crate::updates::{
    Auto, AutoKey, Failure, Finish, Held, Hours, Mint, Package, Reason, Run, RunBy, Unit, Update,
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
                source: format!("{id}-src"),
                version: "1".into(),
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
    for package in &mut gl.packages {
        package.source.clear();
        package.version.clear();
    }
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
    session.host.switches = fake::mint();
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

fn value(pages: &Pages, page_id: &str, row: &str) -> String {
    page(pages, page_id)
        .row(row)
        .and_then(|row| row.value.clone())
        .unwrap_or_default()
}

fn ids<'a>(pages: &'a Pages, page_id: &str) -> Vec<&'a str> {
    page(pages, page_id)
        .rows()
        .map(|row| row.id.as_str())
        .filter(|id| id.starts_with("update:"))
        .collect()
}

fn longer() -> Session {
    let mut session = mint();
    let mut list = session.list().to_vec();
    for index in 0..3 {
        list.push(update(
            &format!("apt:extra{index}"),
            &format!("Extra {index} 1.0"),
            1,
            Unit::Packages,
        ));
    }
    session.apply_updates(
        Updates {
            list,
            problems: Vec::new(),
        },
        at("2026-10-14 13:58"),
    );
    session
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
    let big = format!("packages:apt:007-{}", "x".repeat(48));
    assert_eq!(label(&board.pages, &big, "more"), "Show 41 to 80");
    assert_eq!(
        label(&board.pages, &format!("{big}~4"), "more"),
        "Show 1 to 40"
    );
}

fn sized() -> Session {
    let mut session = mint();
    let mut listed = found(&LISTED, 1, &[("listed-1", 1_000_000)]);
    if let Some(amount) = listed.amount.as_mut() {
        amount.size = Size::Estimate {
            bytes: 1_000_000,
            why: "an estimate",
        };
    }
    session.survey = Some(fake::survey(vec![
        found(&OLD, 1, &[("linux-image-1", 400_000_000)]),
        found(
            &ORPHANS,
            2,
            &[("linux-image-1", 400_000_000), ("libfoo", 2_000_000)],
        ),
        found(&BY_HAND, 1, &[("linux-image-2", 300_000_000)]),
        found(&LAST, 1, &[("core20", 64_000_000)]),
        found(&ALONG, 1, &[("thing-1", 5_000_000)]),
        listed,
        Found {
            leftover: &SCRIPTED,
            amount: Some(Amount::unknown(Why::NeedsAdmin)),
        },
        found(&EXTRA, 0, &[]),
    ]));
    session
}

fn states() -> Vec<Session> {
    let mut borrowed = mint();
    borrowed.borrowed = true;
    borrowed.ask = Some(crate::session::Proposal::Tonight);
    borrowed.in_use = Some(InUse {
        label: "Luna 2".into(),
        bluetooth: true,
    });
    borrowed.host.restart_at = Some(at("2026-10-15 01:30"));
    let mut hours = mint();
    hours.borrowed = true;
    hours.ask = Some(crate::session::Proposal::Hours(None));
    let mut switch = mint();
    switch.borrowed = true;
    switch.ask = Some(crate::session::Proposal::Auto(AutoKey::Packages));
    let mut locked = sized();
    locked.host.admin = false;
    locked.failed = Some(crate::session::Failed {
        updates: vec![locked.list()[0].clone()],
        failure: Failure {
            done: 1,
            reason: Reason::Locked(None),
        },
    });
    let mut problems = Session::new(Journal::default(), false);
    problems.apply_updates(
        Updates {
            list: Vec::new(),
            problems: vec!["the package lists: could not be read".into()],
        },
        at("2026-10-14 13:58"),
    );
    let mut held = mint();
    held.borrowed = true;
    held.ask = Some(crate::session::Proposal::Tonight);
    held.host.restart_at = Some(at("2026-10-15 01:30"));
    held.host.admin = false;
    held.failed = Some(crate::session::Failed {
        updates: vec![held.list()[0].clone()],
        failure: Failure {
            done: 0,
            reason: Reason::Locked(Some("another updater".into())),
        },
    });
    let mut skipping = mint();
    for (update, choice) in [
        ("update:apt:urllib3", "skip"),
        ("update:apt:nvidia", "never"),
    ] {
        let task = press(&mut skipping, update, choice, now())
            .task
            .expect("skip task");
        skipping.done(&task, Ok(()), now());
    }
    vec![
        mint(),
        borrowed,
        held,
        skipping,
        hours,
        switch,
        locked,
        sized(),
        longer(),
        crowded(),
        problems,
        Session::new(Journal::default(), false),
    ]
}

#[test]
fn every_page_is_consistent_in_every_state() {
    for session in states() {
        assert_eq!(broken(&build(&session, now()).pages), Vec::<String>::new());
    }
    let mut session = mint();
    session.host.admin = false;
    session.failed = Some(crate::session::Failed {
        updates: vec![session.list()[0].clone()],
        failure: Failure {
            done: 0,
            reason: Reason::Locked(Some("Mint's Update Manager".into())),
        },
    });
    let pages = build(&session, now()).pages;
    assert!(pages.page("from").is_none());
    assert!(page(&pages, "hours")
        .rows()
        .all(|row| row.enter.is_none() && row.control.is_none()));
    assert_eq!(
        label(&pages, ROOT, "status"),
        "NVIDIA driver 580.178 did not install"
    );
    let auto = page(&pages, "auto");
    assert!(auto
        .row("packages")
        .is_some_and(|row| row.control.is_none() && row.enter.is_none()));
    assert_eq!(value(&pages, "auto", "packages"), "on");
    assert!(auto.row("hours").is_some_and(|row| row.enter.is_some()));
    assert!(auto.row("hold").is_some() && auto.row("guard").is_some());
    assert!(auto.row("default").is_none());
    session.failed = None;
    let pages = build(&session, now()).pages;
    assert_eq!(
        label(&pages, ROOT, "status"),
        "You are not an administrator"
    );
    assert_eq!(value(&pages, ROOT, "status"), "5 waiting");
}

#[test]
fn every_heading_carries_a_sub_header_that_says_what_it_is_for() {
    for session in states() {
        let pages = build(&session, now()).pages;
        for (id, page) in &pages.pages {
            for group in &page.groups {
                let colophon = group
                    .colophon
                    .as_deref()
                    .unwrap_or_else(|| panic!("`{}` on `{id}` has no sub header", group.title));
                assert!(!colophon.trim().is_empty(), "`{}` on `{id}`", group.title);
                assert_eq!(colophon, colophon.to_lowercase(), "on `{id}`");
                for hint in ["esc", "enter", "press"] {
                    assert!(
                        !colophon
                            .split(|c: char| !c.is_alphanumeric())
                            .any(|word| word == hint),
                        "`{colophon}` on `{id}` is a key hint"
                    );
                }
            }
        }
    }
}

#[test]
fn no_heading_row_or_title_names_a_system_a_desktop_or_qol() {
    for session in states() {
        let pages = build(&session, now()).pages;
        for (id, page) in &pages.pages {
            let mut words = vec![page.title.clone()];
            for group in &page.groups {
                words.push(group.title.clone());
                words.extend(group.colophon.clone());
                for row in &group.rows {
                    words.push(row.label.clone());
                    words.extend(row.value.clone());
                    words.extend(row.tag.clone());
                }
            }
            for text in words {
                let lower = text.to_lowercase();
                for name in ["mint", "cinnamon", "ubuntu", "spices", "qol", "left behind"] {
                    assert!(
                        !lower
                            .split(|c: char| !c.is_alphanumeric() && c != ' ')
                            .any(|part| part.split(' ').any(|word| word == name)
                                || (name.contains(' ') && part.contains(name))),
                        "`{text}` on `{id}` names `{name}`"
                    );
                }
            }
        }
    }
}

#[test]
fn the_first_page_reads_like_the_design() {
    let session = mint();
    let pages = build(&session, now()).pages;
    assert_eq!(pages.root, ROOT);
    let root = page(&pages, ROOT);
    assert_eq!(root.title, "Update Manager");
    let titles: Vec<&str> = root
        .groups
        .iter()
        .map(|group| group.title.as_str())
        .collect();
    assert_eq!(titles, ["updates", "in the background"]);
    let rows: Vec<(&str, &str)> = root
        .rows()
        .map(|row| (row.id.as_str(), row.label.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("status", "5 waiting"),
            ("restart", "Restart to finish"),
            ("auto", "Installs on its own"),
            ("cleanup", "Cleanup"),
            ("last", "History"),
        ]
    );
    assert_eq!(
        root.row("status").and_then(|row| row.enter.clone()),
        Some(Enter::Open("waiting".into()))
    );
    assert_eq!(
        detail(&pages, ROOT, "restart"),
        "Linux kernel 6.8.0-147 finishes at the restart"
    );
    assert_eq!(value(&pages, ROOT, "restart"), "");
    assert_eq!(value(&pages, ROOT, "auto"), "on");
    assert!(root.row("cleanup").is_some_and(|row| row.busy));
    assert_eq!(value(&pages, ROOT, "last"), "today 07:45");
    assert!(root.rows().all(|row| row.enter.is_some()));
    assert!(root.rows().all(|row| row.control.is_none()));
}

#[test]
fn the_first_page_keeps_two_groups_and_shows_the_restart_only_while_one_waits() {
    let mut session = mint();
    session.host.finish.clear();
    let pages = build(&session, now()).pages;
    let root = page(&pages, ROOT);
    assert_eq!(root.groups.len(), 2);
    assert_eq!(root.rows().count(), 4);
    assert!(root.row("restart").is_none());
    session.host.restart_at = Some(at("2026-10-15 01:30"));
    let pages = build(&session, now()).pages;
    assert_eq!(label(&pages, ROOT, "restart"), "Restart tonight at 01:30");
    let mut planned = mint();
    planned.host.restart_at = Some(at("2026-10-15 01:30"));
    let pages = build(&planned, now()).pages;
    assert_eq!(label(&pages, ROOT, "restart"), "Restart to finish");
    assert_eq!(value(&pages, ROOT, "restart"), "tonight 01:30");
    planned.host.mint = None;
    planned.host.switches.clear();
    let pages = build(&planned, now()).pages;
    assert!(page(&pages, ROOT).row("auto").is_none());
    assert!(pages.page("auto").is_none());
}

#[test]
fn the_status_row_changes_with_the_state() {
    let looking = Session::new(Journal::default(), false);
    let pages = build(&looking, now()).pages;
    assert_eq!(label(&pages, ROOT, "status"), "Looking for updates");
    assert!(page(&pages, ROOT)
        .row("status")
        .is_some_and(|row| row.busy && row.enter.is_none()));
    assert!(pages.page("waiting").is_none());

    let mut nothing = mint();
    nothing.apply_updates(
        Updates {
            list: Vec::new(),
            problems: Vec::new(),
        },
        at("2026-10-14 13:58"),
    );
    let pages = build(&nothing, now()).pages;
    assert_eq!(label(&pages, ROOT, "status"), "Nothing waiting");
    assert_eq!(value(&pages, ROOT, "status"), "checked 13:58");
    assert!(page(&pages, ROOT)
        .row("status")
        .is_some_and(|row| row.enter.is_none()));

    let mut problems = mint();
    problems.host.problems = vec!["the timer: could not be read".into()];
    let pages = build(&problems, now()).pages;
    assert_eq!(label(&pages, ROOT, "status"), "5 waiting");
    assert_eq!(
        detail(&pages, ROOT, "status"),
        "the timer: could not be read"
    );
    assert_eq!(label(&pages, "waiting", "problem:0"), "the timer");
    assert_eq!(page(&pages, "waiting").groups[0].title, "problems");
}

#[test]
fn the_waiting_list_lists_restarts_first_and_held_last() {
    let pages = build(&mint(), now()).pages;
    assert!(page(&pages, ROOT)
        .rows()
        .all(|row| !row.id.starts_with("update:")));
    assert_eq!(
        ids(&pages, "waiting"),
        [
            "update:apt:nvidia",
            "update:fwupd:dbx",
            "update:apt:urllib3",
            "update:flatpak:gl-nvidia",
            "update:apt:apparmor",
        ]
    );
    assert_eq!(value(&pages, "waiting", "update:apt:nvidia"), "restart");
    assert_eq!(value(&pages, "waiting", "update:apt:urllib3"), "");
    assert_eq!(
        value(&pages, "waiting", "update:apt:apparmor"),
        "rolling out slowly"
    );
    let waiting = page(&pages, "waiting");
    assert!(waiting
        .row("update:apt:apparmor")
        .is_some_and(|row| row.dim));
    assert!(waiting.row("skipped").is_none());
    assert_eq!(label(&pages, "waiting", "install"), "5 waiting");
    assert_eq!(
        waiting.row("install").and_then(|row| row.control.clone()),
        Some(Control::Button("Install all".into()))
    );
    assert_eq!(
        waiting.row("install").and_then(|row| row.enter.clone()),
        Some(Enter::Open(super::updates::install_all(&mint())))
    );
}

#[test]
fn every_update_waits_on_the_list_the_status_row_opens() {
    let pages = build(&longer(), now()).pages;
    assert_eq!(ids(&pages, ROOT).len(), 0);
    assert_eq!(label(&pages, ROOT, "status"), "8 waiting");
    let waiting = page(&pages, "waiting");
    assert!(waiting.groups.iter().all(|group| group.colophon.is_some()));
    assert_eq!(label(&pages, "waiting", "install"), "8 waiting");
    assert_eq!(ids(&pages, "waiting").len(), 8);
    assert!(waiting
        .row("update:apt:nvidia")
        .is_some_and(|row| row.tag.as_deref() == Some("Update Manager")));
}

#[test]
fn one_shared_updater_is_said_once() {
    let mut session = longer();
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
    assert!(page(&pages, "waiting")
        .rows()
        .filter(|row| row.id.starts_with("update:"))
        .all(|row| row.tag.is_none()));
}

#[test]
fn an_update_page_warns_then_installs_skips_or_never_updates() {
    let mut session = mint();
    let pages = build(&session, now()).pages;
    let nvidia = page(&pages, "update:apt:nvidia");
    assert_eq!(nvidia.title, "nvidia driver 580.178");
    assert!(nvidia.groups.iter().all(|group| group.colophon.is_some()));
    let rows: Vec<&str> = nvidia.rows().map(|row| row.id.as_str()).collect();
    assert_eq!(
        rows,
        ["restart", "caution", "install", "skip", "never", "packages"]
    );
    assert_eq!(
        label(&pages, "update:apt:nvidia", "restart"),
        "Finishes at a restart"
    );
    assert_eq!(
        label(&pages, "update:apt:nvidia", "caution"),
        "Games opened before the restart may fail"
    );
    assert_eq!(value(&pages, "update:apt:nvidia", "install"), "20 packages");
    assert_eq!(label(&pages, "update:apt:nvidia", "skip"), "Skip 580.178");
    assert_eq!(value(&pages, "update:apt:nvidia", "packages"), "20");
    assert_eq!(
        page(&pages, "packages:apt:nvidia")
            .rows()
            .filter(|row| row.id.starts_with("package:"))
            .count(),
        20
    );
    let urllib3 = page(&pages, "update:apt:urllib3");
    let rows: Vec<&str> = urllib3.rows().map(|row| row.id.as_str()).collect();
    assert_eq!(rows, ["install", "skip", "never"]);
    assert_eq!(value(&pages, "update:apt:urllib3", "install"), "96 kB");
    assert_eq!(label(&pages, "update:apt:urllib3", "skip"), "Skip 2.0.7");
    assert!(page(&pages, "update:flatpak:gl-nvidia")
        .row("skip")
        .is_none());
    let installed = press(&mut session, "update:apt:urllib3", "install", now());
    assert_eq!(installed.go, None);
    assert!(matches!(installed.task, Some(Task::Install(ref list)) if list.len() == 1));
}

#[test]
fn skipped_updates_hide_until_they_update_again() {
    let mut session = mint();
    let skip = press(&mut session, "update:apt:urllib3", "skip", now());
    assert_eq!(skip.go, Some(Go::Back));
    let task = skip.task.expect("skip task");
    assert_eq!(
        task,
        Task::Skips {
            add: vec!["apt:urllib3-src=1".into()],
            remove: Vec::new(),
            system: true,
        }
    );
    session.done(&task, Ok(()), now());
    let pages = build(&session, now()).pages;
    assert!(session.update("apt:urllib3").is_none());
    assert_eq!(value(&pages, "waiting", "skipped"), "1");
    assert_eq!(
        label(&pages, "skipped", "skip:apt:urllib3-src=1"),
        "apt:urllib3-src"
    );
    let never = press(&mut session, "update:apt:nvidia", "never", now());
    let task = never.task.expect("never task");
    assert_eq!(
        task,
        Task::Skips {
            add: vec!["apt:nvidia-src".into()],
            remove: Vec::new(),
            system: true,
        }
    );
    session.done(&task, Ok(()), now());
    let pages = build(&session, now()).pages;
    assert!(session.update("apt:nvidia").is_none());
    assert_eq!(value(&pages, "waiting", "skipped"), "2");
    let titles: Vec<&str> = page(&pages, "skipped")
        .groups
        .iter()
        .map(|group| group.title.as_str())
        .collect();
    assert_eq!(titles, ["never", "this version"]);
    let again = press(&mut session, "skipped", "never:apt:nvidia-src", now());
    let task = again.task.expect("update again task");
    session.done(&task, Ok(()), now());
    assert!(session.update("apt:nvidia").is_some());
    let mut locked = mint();
    locked.host.admin = false;
    let Some(Task::Skips { system, .. }) =
        press(&mut locked, "update:apt:urllib3", "never", now()).task
    else {
        panic!("never must change the skipped list");
    };
    assert!(!system);
    locked.done(
        &Task::Skips {
            add: Vec::new(),
            remove: Vec::new(),
            system,
        },
        Err(Refusal::Denied),
        now(),
    );
    let pages = build(&locked, now()).pages;
    assert_eq!(broken(&pages), Vec::<String>::new());
    assert!(page(&pages, "skipped").row("error").is_some());
}

#[test]
fn install_asks_first_then_starts_one_job() {
    let mut session = mint();
    let pages = build(&session, now()).pages;
    let all = super::updates::install_all(&session);
    let install = page(&pages, &all);
    assert_eq!(install.groups[0].title, "before you install");
    assert_eq!(
        label(&pages, &all, "restart"),
        "NVIDIA driver 580.178 and the Secure Boot list"
    );
    assert_eq!(label(&pages, &all, "held"), "AppArmor 4.0.1 stays held");
    assert_eq!(label(&pages, &all, "go"), "Install 30 updates");
    let outcome = press(&mut session, &all, "go", now());
    assert_eq!(outcome.go, Some(Go::Back));
    let Some(Task::Install(list)) = outcome.task else {
        panic!("install must start a job");
    };
    assert_eq!(list.len(), 4);
    let pages = build(&session, now()).pages;
    assert_eq!(label(&pages, ROOT, "status"), "Waiting for the password");
    assert_eq!(value(&pages, ROOT, "status"), "30 updates");
    assert_eq!(
        value(&pages, "waiting", "update:apt:nvidia"),
        "waits for the password"
    );
    session.progress(0);
    let pages = build(&session, now()).pages;
    assert_eq!(label(&pages, ROOT, "status"), "Installing 30 updates");
    assert!(pages.page(&all).is_none());
    assert_eq!(
        value(&pages, "waiting", "update:apt:nvidia"),
        "installing, 0 of 20 packages"
    );
    let again = press(&mut session, "update:apt:urllib3", "install", now());
    assert!(matches!(again.notice, Some(Notice::Failure(_))));
}

#[test]
fn install_all_starts_at_once_when_nothing_needs_a_warning() {
    let mut session = mint();
    let list: Vec<Update> = session
        .list()
        .iter()
        .filter(|update| !update.restart)
        .cloned()
        .collect();
    session.apply_updates(
        Updates {
            list,
            problems: Vec::new(),
        },
        at("2026-10-14 13:58"),
    );
    let pages = build(&session, now()).pages;
    assert_eq!(
        page(&pages, "waiting")
            .row("install")
            .and_then(|row| row.enter.clone()),
        Some(Enter::Press)
    );
    let outcome = press(&mut session, "waiting", "install", now());
    assert!(matches!(outcome.task, Some(Task::Install(ref list)) if list.len() == 2));
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
    assert_eq!(label(&pages, "waiting", "install"), "5 waiting");
    let row = page(&pages, "waiting")
        .row("update:apt:nvidia")
        .expect("nvidia row");
    assert!(!row.dim);
    assert_eq!(
        label(&pages, "update:apt:nvidia", "release"),
        "Its hold comes off"
    );
    let all = super::updates::install_all(&session);
    assert_eq!(
        label(&pages, &all, "release:apt:nvidia"),
        "Its hold comes off"
    );
    assert_eq!(label(&pages, &all, "held"), "AppArmor 4.0.1 stays held");
    assert!(page(&pages, "update:apt:apparmor").row("install").is_none());
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
    press(&mut session, "update:apt:nvidia", "install", now());
    session.installed(Err(Failure {
        done: 0,
        reason: Reason::Locked(Some("Mint's Update Manager".into())),
    }));
    let pages = build(&session, now()).pages;
    assert_eq!(
        label(&pages, ROOT, "status"),
        "NVIDIA driver 580.178 did not install"
    );
    assert_eq!(
        detail(&pages, ROOT, "status"),
        "Another installer was already running. Nothing changed; it still waits."
    );
    assert_eq!(
        page(&pages, ROOT)
            .row("status")
            .and_then(|row| row.enter.clone()),
        Some(Enter::Open("waiting".into()))
    );
    assert_eq!(
        page(&pages, "waiting")
            .row("failed")
            .and_then(|row| row.enter.clone()),
        Some(Enter::Open("failed".into()))
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
        "After the automatic update at 00:37 ends. Stays set if Update Manager stops."
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
fn pause_and_wait_live_with_the_installs_they_hold() {
    let mut session = mint();
    let auto = page(&build(&session, now()).pages, "auto").clone();
    let groups: Vec<&str> = auto
        .groups
        .iter()
        .map(|group| group.title.as_str())
        .collect();
    assert_eq!(groups, ["what", "when"]);
    let rows: Vec<(&str, &str)> = auto
        .rows()
        .map(|row| (row.id.as_str(), row.label.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("packages", "System packages"),
            ("flatpak", "Flatpak"),
            ("spices", "Desktop add-ons"),
            ("hours", "Between"),
            ("hold", "Pause all"),
            ("guard", "Wait while sound plays"),
        ]
    );
    let pages = build(&session, now()).pages;
    assert_eq!(value(&pages, "auto", "hours"), "00:00 and 01:00");
    assert_eq!(detail(&pages, "auto", "packages"), "In the update hours");
    assert_eq!(detail(&pages, "auto", "flatpak"), "When you log in");
    assert_eq!(detail(&pages, "auto", "hold"), "");
    assert_eq!(
        page(&pages, "auto")
            .row("guard")
            .and_then(|row| row.control.clone()),
        Some(Control::Toggle(false))
    );
    let outcome = press(&mut session, "auto", "guard", now());
    assert_eq!(outcome.task, Some(Task::Rules));
    assert!(session.rules.guard);
    press(&mut session, "auto", "hold", now());
    session.rules.helper = Helper::Ready;
    let pages = build(&session, now()).pages;
    assert_eq!(value(&pages, ROOT, "auto"), "paused");
    if let Some(mint) = session.host.mint.as_mut() {
        mint.ours = true;
        mint.hours = Some(Hours {
            from: crate::updates::Clock::hm(2, 0),
            until: crate::updates::Clock::hm(3, 0),
            ..MINT_DEFAULT
        });
    }
    let pages = build(&session, now()).pages;
    assert_eq!(label(&pages, "auto", "default"), "Set back to the default");
    let back = press(&mut session, "auto", "default", now());
    assert!(matches!(back.task, Some(Task::Hours { hours: None, .. })));
}

#[test]
fn a_planned_restart_is_cancelled_where_it_is_set() {
    let mut session = mint();
    session.host.restart_at = Some(at("2026-10-15 01:30"));
    let pages = build(&session, now()).pages;
    assert!(pages.page("changes").is_none());
    assert_eq!(
        page(&pages, ROOT)
            .row("restart")
            .and_then(|row| row.enter.clone()),
        Some(Enter::Open("restart".into()))
    );
    let cancel = press(&mut session, "restart", "tonight", now());
    assert_eq!(cancel.task, Some(Task::Restart(None)));
}

#[test]
fn a_pause_the_password_prompt_refused_says_so_on_its_row() {
    let mut session = mint();
    let held = press(&mut session, "auto", "hold", now());
    assert_eq!(held.task, Some(Task::Rules));
    session.rules.helper = Helper::Starting;
    let pages = build(&session, now()).pages;
    assert_eq!(value(&pages, ROOT, "auto"), "on");
    assert_eq!(
        detail(&pages, "auto", "hold"),
        "Waiting for the password, to hold the automatic update"
    );
    session.rules.helper = Helper::Failed(Refusal::Dismissed);
    let pages = build(&session, now()).pages;
    assert_eq!(value(&pages, ROOT, "auto"), "on");
    assert_eq!(
        detail(&pages, "auto", "hold"),
        "The password prompt was closed."
    );
}

#[test]
fn rules_the_password_prompt_refused_switch_off_and_retry() {
    let mut session = mint();
    press(&mut session, "auto", "guard", now());
    press(&mut session, "auto", "hold", now());
    session.rules.helper = Helper::Failed(Refusal::TimedOut);
    let pages = build(&session, now()).pages;
    assert_eq!(
        detail(&pages, "auto", "guard"),
        "The password prompt timed out."
    );
    assert_eq!(
        page(&pages, "auto")
            .row("hold")
            .and_then(|row| row.control.clone()),
        Some(Control::Toggle(true))
    );
    assert_eq!(label(&pages, "auto", "retry"), "Try again");
    assert_eq!(
        press(&mut session, "auto", "hold", now()).task,
        Some(Task::Rules)
    );
    assert!(!session.rules.hold);
    assert!(!session.want_helper());
    let pages = build(&session, now()).pages;
    assert_eq!(detail(&pages, "auto", "hold"), "");
    assert!(page(&pages, "auto").row("retry").is_some());
    press(&mut session, "auto", "retry", now());
    assert_eq!(session.rules.helper, Helper::Off);
    assert!(session.rules.guard);
    assert!(session.want_helper());
    session.rules.helper = Helper::Failed(Refusal::Dismissed);
    press(&mut session, "auto", "guard", now());
    assert!(!session.rules.guard);
    assert_eq!(session.rules.helper, Helper::Off);
    assert!(!session.want_helper());
    let pages = build(&session, now()).pages;
    assert!(page(&pages, "auto").row("retry").is_none());
    assert_eq!(
        page(&pages, "auto")
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

#[test]
fn cleanup_lists_the_largest_kinds_first_and_hides_the_empty_ones() {
    let session = sized();
    let pages = build(&session, now()).pages;
    assert_eq!(value(&pages, ROOT, "cleanup"), "about 772 MB and more");
    let cleanup = page(&pages, "cleanup");
    let rows: Vec<&str> = cleanup.rows().map(|row| row.id.as_str()).collect();
    assert_eq!(
        rows,
        [
            "all",
            "kind:orphans",
            "kind:kernels",
            "kind:kernels_by_hand",
            "kind:last",
            "kind:along",
            "kind:listed",
            "more",
            "kernels",
            "by_hand",
        ]
    );
    assert_eq!(label(&pages, "cleanup", "all"), "Remove all");
    assert_eq!(
        detail(&pages, "cleanup", "all"),
        "About 403 MB and more. Leaves out what asks first"
    );
    assert_eq!(
        cleanup.row("all").and_then(|row| row.control.clone()),
        Some(Control::Destroy {
            verb: "remove".into(),
            ask: "remove all".into(),
        })
    );
    assert_eq!(
        detail(&pages, "cleanup", "kind:listed"),
        "About 1.0 MB, 1 listed"
    );
    assert_eq!(
        detail(&pages, "cleanup", "kind:orphans"),
        "402 MB, 2 packages"
    );
    assert_eq!(
        detail(&pages, "cleanup", "kind:kernels"),
        "400 MB, 1 old kernel"
    );
    assert_eq!(
        detail(&pages, "cleanup", "kind:last"),
        "64 MB, 1 old version. Asks first: they are the last way back to the version before"
    );
    assert_eq!(label(&pages, "cleanup", "more"), "1 more");
    assert!(detail(&pages, "cleanup:more", "kind:scripted").starts_with("Size unknown. "));
    assert!(cleanup
        .rows()
        .filter(|row| matches!(row.control, Some(Control::Destroy { .. })))
        .all(|row| row.value.is_none()));
    assert!(cleanup.row("kind:extra").is_none());
    assert!(cleanup
        .rows()
        .filter(|row| row.id.starts_with("kind:"))
        .all(|row| matches!(row.control, Some(Control::Destroy { .. }))));
}

#[test]
fn cleanup_removes_with_the_consent_the_command_line_asks_for() {
    let mut session = sized();
    let all = press(&mut session, "cleanup", "all", now());
    assert_eq!(
        all.task,
        Some(Task::Remove {
            kinds: vec![&ORPHANS, &OLD, &LISTED, &SCRIPTED],
            consent: Consent::All,
        })
    );
    let again = press(&mut session, "cleanup", "kind:kernels", now());
    assert!(matches!(again.notice, Some(Notice::Failure(_))));
    let install = press(&mut session, "update:apt:urllib3", "install", now());
    assert!(matches!(install.notice, Some(Notice::Failure(_))));
    assert!(session.job.is_none());
    assert!(session
        .apply(&crate::session::Act::RestartNow, now())
        .is_err());
    let pages = build(&session, now()).pages;
    assert!(page(&pages, "cleanup")
        .row("all")
        .is_some_and(|row| row.busy));
    session.done(
        &all.task.expect("task"),
        Err(Refusal::Failed("orphans: apt is locked".into())),
        now(),
    );
    assert!(!session.busy.contains(&Op::Remove));
    let pages = build(&session, now()).pages;
    assert_eq!(detail(&pages, "cleanup", "error"), "orphans: apt is locked");
    let one = press(&mut session, "cleanup", "kind:kernels", now());
    assert_eq!(
        one.task,
        Some(Task::Remove {
            kinds: vec![&OLD],
            consent: Consent::All,
        })
    );
    let mut session = sized();
    let asked = press(&mut session, "cleanup", "kind:last", now());
    assert_eq!(
        asked.task,
        Some(Task::Remove {
            kinds: vec![&LAST],
            consent: Consent::Asked,
        })
    );
    let mut session = sized();
    session.job = Some(crate::session::Job {
        updates: Vec::new(),
        done: 0,
        started: false,
    });
    let refused = press(&mut session, "cleanup", "all", now());
    assert!(matches!(refused.notice, Some(Notice::Failure(_))));
    let mut session = sized();
    session.busy.insert(Op::RestartNow);
    let restarting = press(&mut session, "cleanup", "all", now());
    assert!(matches!(restarting.notice, Some(Notice::Failure(_))));
    assert!(restarting.task.is_none());
    assert!(!session.busy.contains(&Op::Remove));
}

#[test]
fn a_failed_hours_change_shows_on_the_hours_page() {
    let mut session = mint();
    session
        .errors
        .insert(Op::Hours, "the password was not accepted".into());
    let pages = build(&session, now()).pages;
    assert_eq!(
        label(&pages, "hours", "error"),
        "Could not change the hours"
    );
    assert_eq!(
        detail(&pages, "hours", "error"),
        "the password was not accepted"
    );
    assert!(page(&pages, "hours")
        .row("error")
        .is_some_and(|row| row.attention));
}

#[test]
fn cleanup_before_the_first_measure_says_it_is_measuring() {
    let pages = build(&mint(), now()).pages;
    assert!(page(&pages, "cleanup")
        .row("measuring")
        .is_some_and(|row| row.busy));
    assert!(page(&pages, "cleanup").row("all").is_none());
    let mut nothing = mint();
    nothing.survey = Some(fake::survey(vec![found(&OLD, 0, &[])]));
    let pages = build(&nothing, now()).pages;
    assert_eq!(value(&pages, ROOT, "cleanup"), "nothing to remove");
    assert!(page(&pages, "cleanup").row("nothing").is_some());
}

#[test]
fn old_kernels_switch_through_tasks_on_the_cleanup_page() {
    let mut session = mint();
    let pages = build(&session, now()).pages;
    assert_eq!(
        label(&pages, "cleanup", "kernels"),
        "Old kernels and leftovers"
    );
    assert_eq!(
        detail(&pages, "cleanup", "kernels"),
        "Weekly and 3 hours after each startup. Also removes other packages nothing needs."
    );
    assert_eq!(
        detail(&pages, "cleanup", "by_hand"),
        "A kernel installed from Mint's Update Manager window is never removed."
    );
    let switch = press(&mut session, "cleanup", "by_hand", now());
    let task = switch.task.expect("task");
    assert_eq!(
        task,
        Task::Auto {
            key: AutoKey::Switch(&MINT_BY_HAND),
            on: true,
            was: false,
        }
    );
    session.done(&task, Ok(()), now());
    session.host.switches[1].state = State::On { idle: false };
    let pages = build(&session, now()).pages;
    assert_eq!(broken(&pages), Vec::<String>::new());
    assert_eq!(
        page(&pages, "cleanup")
            .row("by_hand")
            .and_then(|row| row.control.clone()),
        Some(Control::Toggle(true))
    );
}

#[test]
fn old_kernels_read_like_each_platform_removes_them() {
    let mut session = mint();
    let mut by_hand = found(&BY_HAND, 1, &[("linux-image-1", 1)]);
    if let Some(amount) = by_hand.amount.as_mut() {
        amount.present = 3;
    }
    session.survey = Some(fake::survey(vec![by_hand]));
    let pages = build(&session, now()).pages;
    assert_eq!(
        detail(&pages, "cleanup", "by_hand"),
        "3 kernels installed by hand are never removed.",
        "the page counts every kernel installed by hand but the running one, as main did"
    );

    let unattended = Live {
        switch: &UNATTENDED_KERNELS,
        state: State::On { idle: false },
    };
    session.host.switches = vec![unattended];
    let pages = build(&session, now()).pages;
    assert_eq!(broken(&pages), Vec::<String>::new());
    assert_eq!(label(&pages, "cleanup", "kernels"), "Old kernels");
    assert!(page(&pages, "cleanup").row("by_hand").is_none());

    session.host.switches = vec![Live {
        state: State::On { idle: true },
        ..unattended
    }];
    let pages = build(&session, now()).pages;
    assert_eq!(
        detail(&pages, "cleanup", "kernels"),
        "Automatic updates are off here, so this never runs."
    );

    session.host.switches = vec![Live {
        state: State::Unknown("apt's settings could not be read"),
        ..unattended
    }];
    let pages = build(&session, now()).pages;
    assert_eq!(broken(&pages), Vec::<String>::new());
    assert_eq!(
        detail(&pages, "cleanup", "kernels"),
        "Old kernels stay until you remove them."
    );
    assert_eq!(
        page(&pages, "cleanup")
            .row("kernels")
            .and_then(|row| row.control.clone()),
        Some(Control::Toggle(false))
    );

    session.host.switches.clear();
    let pages = build(&session, now()).pages;
    assert_eq!(broken(&pages), Vec::<String>::new());
    assert!(pages
        .page("cleanup")
        .is_none_or(|page| page.row("kernels").is_none()));

    session.host.switches = vec![unattended];
    session.host.admin = false;
    let pages = build(&session, now()).pages;
    assert_eq!(broken(&pages), Vec::<String>::new());
    assert_eq!(value(&pages, "cleanup", "kernels"), "removed with updates");
    assert!(page(&pages, "cleanup")
        .row("kernels")
        .is_some_and(|row| row.control.is_none()));
}

#[test]
fn a_borrowed_computer_asks_before_changing_the_owners_kernel_removal() {
    let mut session = mint();
    session.borrowed = true;
    let outcome = press(&mut session, "cleanup", "kernels", now());
    assert_eq!(outcome.go, Some(Go::Open("ask".into())));
    assert!(outcome.task.is_none());
    let pages = build(&session, now()).pages;
    assert_eq!(
        page(&pages, "ask").groups[0].title,
        "change the owner's old kernel removal switch?"
    );
    assert_eq!(value(&pages, "ask", "after"), "off");
    let change = press(&mut session, "ask", "change", now());
    assert_eq!(
        change.task,
        Some(Task::Auto {
            key: AutoKey::Switch(&MINT_KERNELS),
            on: false,
            was: true,
        })
    );
}

#[test]
fn an_extra_kind_in_the_survey_adds_no_row() {
    let mut session = mint();
    let before = build(&session, now()).pages;
    let rows = |pages: &Pages, id: &str| -> Vec<String> {
        page(pages, id)
            .groups
            .iter()
            .flat_map(|group| {
                group
                    .rows
                    .iter()
                    .map(|row| format!("{}={}", row.id, row.label))
            })
            .collect()
    };
    session.survey = Some(fake::survey(vec![found(&EXTRA, 3, &[("extra-1", 10)])]));
    let after = build(&session, now()).pages;
    assert_eq!(rows(&after, ROOT), rows(&before, ROOT), "page `{ROOT}`");
    assert_eq!(
        rows(&after, "cleanup")
            .iter()
            .filter(|row| row.starts_with("kind:"))
            .collect::<Vec<_>>(),
        ["kind:extra=extra"]
    );
    assert_eq!(broken(&after), Vec::<String>::new());
}
