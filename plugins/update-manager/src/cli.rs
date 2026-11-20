use std::process::ExitCode;

use anyhow::{Context, Result};
use chrono::Local;
use qol_config::text::{format_bytes, plural};
use qol_headless::{
    Command, CommandContext, DoctorCheck, DoctorCheckResult, Execution, HeadlessApp,
};

use crate::cleanup::{
    self, asks, leftovers, primary, Consent, Found, Leftover, Live, Origin, Size, State, Survey,
};
use crate::platform::Refusal;
use crate::updates;
use crate::PLUGIN_ID;

const PLATFORM_SUPPORTED: &str = "platform_supported";
const REQUIRED_BINARIES: &str = "required_binaries";
const REQUIRED: [&str; 4] = ["apt-get", "apt-cache", "pkexec", "systemctl"];
const OPTIONAL: [&str; 2] = ["flatpak", "fwupdmgr"];

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    HeadlessApp::new(PLUGIN_ID, PLUGIN_ID)
        .about("See what updates wait, install them when you choose, and restart when it suits you.")
        .command(
            Command::new("status")
                .about("List the updates that wait, what finishes at a restart, what is skipped, and whether old kernels are removed.")
                .usage(format!("{PLUGIN_ID} status"))
                .output("One line per waiting update, then what a restart finishes, the skipped entries, and an `Old kernels:` line.")
                .exit_behavior("Exits 64 on a usage error; 0 otherwise, with read problems on stderr.")
                .run_result(status),
        )
        .command(
            Command::new("cleanup")
                .about("List what updates left behind, the space it takes, and what removes it on its own; or remove it.")
                .usage(format!("{PLUGIN_ID} cleanup [remove <kind>|all] [--confirm]"))
                .detail("`remove all` leaves out kinds that are the last way back or were installed by hand; name one of those and add --confirm to remove it. A kind that needs an administrator asks for the password, and closing that prompt stops `remove all`.")
                .output("One line per kind with its id and size, the total, when it was measured, then the switches that remove leftovers on their own. `remove` prints one line per kind removed.")
                .exit_behavior("Exits 64 on a usage error, 1 when a removal is refused or fails, 0 otherwise.")
                .run_result(cleanup_command),
        )
        .command(
            Command::new("press")
                .about("Do what a row on an Update Manager page does.")
                .usage(format!("{PLUGIN_ID} press"))
                .detail("The running daemon answers presses from the settings panel; this command only says so.")
                .exit_behavior("Always exits non-zero: a press needs the daemon.")
                .run_result(press),
        )
        .command(
            Command::new("settings")
                .about("Open the Update Manager settings.")
                .usage(format!("{PLUGIN_ID} settings"))
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero if the settings cannot be opened.")
                .run_result(settings),
        )
        .doctor_checks(checks())
}

fn no_args(context: &CommandContext, usage: &str) -> Option<Execution> {
    context.args().first().map(|token| {
        let problem = if token.starts_with('-') {
            format!("unknown flag `{token}`")
        } else {
            format!("unexpected argument `{token}`")
        };
        Execution::usage(format!("{problem}\n{usage}"))
    })
}

fn status(context: &CommandContext) -> Result<Execution> {
    if let Some(usage) = no_args(context, &format!("{PLUGIN_ID} status")) {
        return Ok(usage);
    }
    let found = updates::read_updates();
    let host = updates::read_host();
    let list = updates::visible(&found.list, &host.skips);
    let mut out = String::new();
    let total: usize = list.iter().map(|update| update.count()).sum();
    out.push_str(&format!("{total} waiting\n"));
    for update in &list {
        out.push_str(&format!(
            "  {} [{}]: {}\n",
            update.label,
            update.tag,
            update.wait()
        ));
    }
    for row in &host.finish {
        out.push_str(&format!("Finishes at a restart: {}\n", row.label));
    }
    if !host.skips.is_empty() {
        out.push_str(&format!("Skipped: {}\n", host.skips.join(", ")));
    }
    out.push_str(&format!("Old kernels: {}\n", kernels(&host.switches)));
    let problems: Vec<String> = found.problems.into_iter().chain(host.problems).collect();
    Ok(Execution::new(out, problems.join("\n"), 0))
}

fn kernels(switches: &[Live]) -> String {
    let Some(main) = primary(switches) else {
        return "kept, nothing removes them on its own here".into();
    };
    match main.state {
        State::Unknown(reason) => format!("unknown, {reason}"),
        State::Off | State::On { idle: true } => "kept".into(),
        State::On { idle: false } => {
            let also: String = switches
                .iter()
                .filter(|live| live.state.on())
                .flat_map(|live| live.switch.covers.iter())
                .filter(|kind| kind.origin == Origin::ByHand)
                .map(|kind| format!(", {} too", kind.noun.many))
                .collect();
            format!("{}{also}", main.switch.when)
        }
    }
}

fn cleanup_command(context: &CommandContext) -> Result<Execution> {
    let usage = format!("{PLUGIN_ID} cleanup [remove <kind>|all] [--confirm]");
    let args: Vec<&str> = context.args().iter().map(String::as_str).collect();
    let (target, confirm) = match args.as_slice() {
        [] => {
            let survey = cleanup::survey(Local::now().naive_local());
            return Ok(Execution::success(listing(
                &survey,
                &cleanup::read_switches(),
            )));
        }
        ["remove", target] => (*target, false),
        ["remove", target, "--confirm"] | ["remove", "--confirm", target] => (*target, true),
        _ => {
            return Ok(Execution::usage(format!(
                "expected nothing or `remove <kind>`\n{usage}"
            )))
        }
    };
    let (kinds, consent) = match choose(target, confirm, leftovers()) {
        Ok(chosen) => chosen,
        Err(problem) => return Ok(Execution::usage(format!("{problem}\n{usage}"))),
    };
    let mut out = String::new();
    let mut failed = Vec::new();
    for kind in kinds {
        let found = cleanup::find(kind);
        if empty(&found) {
            if target != "all" {
                out.push_str(&format!("Nothing to remove: {}\n", kind.name));
            }
            continue;
        }
        match cleanup::remove(&found, consent) {
            Ok(()) => out.push_str(&format!("Removed: {}\n", kind.name)),
            Err(refusal) if asks(kind) && !confirm => {
                failed.push(format!("{}: {refusal}; add --confirm", kind.name))
            }
            Err(refusal @ (Refusal::Dismissed | Refusal::Denied | Refusal::TimedOut)) => {
                failed.push(format!(
                    "{}: {refusal}, so nothing more was removed",
                    kind.name
                ));
                break;
            }
            Err(refusal) => failed.push(format!("{}: {refusal}", kind.name)),
        }
    }
    let code = u8::from(!failed.is_empty());
    Ok(Execution::new(out, failed.join("\n"), code))
}

fn choose(
    target: &str,
    confirm: bool,
    table: &'static [&'static Leftover],
) -> Result<(Vec<&'static Leftover>, Consent), String> {
    if target == "all" {
        if confirm {
            return Err(
                "--confirm goes with one kind; `remove all` leaves out the kinds that ask first"
                    .into(),
            );
        }
        let kinds = table.iter().copied().filter(|kind| !asks(kind)).collect();
        return Ok((kinds, Consent::All));
    }
    let kind = table
        .iter()
        .copied()
        .find(|kind| kind.id == target)
        .ok_or_else(|| format!("unknown kind `{target}`; `{PLUGIN_ID} cleanup` lists them"))?;
    let consent = if confirm {
        Consent::Asked
    } else {
        Consent::All
    };
    Ok((vec![kind], consent))
}

fn empty(found: &Found) -> bool {
    found
        .amount
        .as_ref()
        .is_none_or(|amount| amount.count == 0 && !matches!(amount.size, Size::Unknown(_)))
}

fn listing(survey: &Survey, switches: &[Live]) -> String {
    let mut out = String::new();
    for found in &survey.found {
        let kind = found.leftover;
        let mut line = format!("{} [{}]: {}", kind.name, kind.id, amount(found));
        if asks(kind) {
            line.push_str(", asks first");
        }
        for along in kind.along {
            line.push_str(&format!(", takes {} along", along.name.to_lowercase()));
        }
        out.push_str(&line);
        out.push('\n');
    }
    let total = survey.total();
    out.push_str(&format!(
        "Altogether: {}{}{}\n",
        if total.estimate { "about " } else { "" },
        format_bytes(total.bytes),
        match total.unknown {
            0 => String::new(),
            unknown => format!(", and {} of unknown size", plural(unknown, "kind", "kinds")),
        }
    ));
    out.push_str(&format!("Measured at {}\n", survey.at.format("%H:%M")));
    for live in switches {
        out.push_str(&format!(
            "Switch, {}: {}: {}\n",
            live.switch.owner.title,
            live.switch.name,
            match live.state {
                State::On { idle: false } => "on".to_string(),
                State::On { idle: true } => "on, but never runs".to_string(),
                State::Off => "off".to_string(),
                State::Unknown(reason) => format!("unknown, {reason}"),
            }
        ));
    }
    out
}

fn amount(found: &Found) -> String {
    let kind = found.leftover;
    let Some(amount) = found.amount.as_ref() else {
        return format!(
            "not here, {}",
            kind.requires
                .missing()
                .unwrap_or_else(|| "it went away".into())
        );
    };
    let count = plural(amount.count, kind.noun.one, kind.noun.many);
    match &amount.size {
        Size::Exact(size) => format!("{count}, {}", format_bytes(*size)),
        Size::Estimate { bytes, why } => format!("{count}, about {} ({why})", format_bytes(*bytes)),
        Size::Unknown(why) => format!("size unknown, {}", why.reason()),
    }
}

fn press(_context: &CommandContext) -> Result<Execution> {
    Ok(Execution::runtime_error(
        "press is answered by the running Update Manager daemon; open its settings from qol-tray",
    ))
}

fn settings(context: &CommandContext) -> Result<Execution> {
    if let Some(usage) = no_args(context, &format!("{PLUGIN_ID} settings")) {
        return Ok(usage);
    }
    qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID)
        .context("failed to open the Update Manager settings")?;
    Ok(Execution::success(String::new()))
}

fn checks() -> Vec<DoctorCheck> {
    let mut checks = vec![
        DoctorCheck::new(
            PLATFORM_SUPPORTED,
            "Answer whether Update Manager can read this computer's updaters.",
            || Ok(platform_supported()),
        ),
        DoctorCheck::new(
            REQUIRED_BINARIES,
            "Answer whether the programs Update Manager runs are installed.",
            || Ok(required_binaries()),
        ),
    ];
    checks.extend(leftovers().iter().copied().map(kind_check));
    checks
}

fn kind_check(kind: &'static Leftover) -> DoctorCheck {
    let id = format!("cleanup_{}", kind.id);
    DoctorCheck::new(
        id.clone(),
        format!(
            "Answer whether qol can measure {} here; `{PLUGIN_ID} cleanup` measures them. {}",
            kind.name.to_lowercase(),
            kind.what
        ),
        move || Ok(kind_result(&id, kind, kind.requires.missing())),
    )
}

fn kind_result(id: &str, kind: &Leftover, missing: Option<String>) -> DoctorCheckResult {
    let message = match missing {
        Some(missing) => format!("{}: not here, {missing}", kind.name),
        None => format!("{}: `{PLUGIN_ID} cleanup` measures them", kind.name),
    };
    DoctorCheckResult::ok(id, message)
}

fn platform_supported() -> DoctorCheckResult {
    match updates::supported() {
        Err(problem) => DoctorCheckResult::fail(PLATFORM_SUPPORTED, problem),
        Ok(()) if updates::is_mint() => DoctorCheckResult::ok(
            PLATFORM_SUPPORTED,
            "Reads APT, Flatpak, firmware and Mint's automatic upgrades",
        ),
        Ok(()) => DoctorCheckResult::warn(
            PLATFORM_SUPPORTED,
            "Mint's Update Manager is not installed: updates list, and nothing installs on its own",
        ),
    }
}

fn required_binaries() -> DoctorCheckResult {
    let required = updates::missing(&REQUIRED);
    if !required.is_empty() {
        return DoctorCheckResult::fail(
            REQUIRED_BINARIES,
            format!("{} missing, so nothing installs", required.join(", ")),
        )
        .with_fix(format!("Install {}", required.join(" and ")));
    }
    let optional = updates::missing(&OPTIONAL);
    if !optional.is_empty() {
        return DoctorCheckResult::warn(
            REQUIRED_BINARIES,
            format!(
                "{} missing, so those updates are not listed",
                optional.join(", ")
            ),
        );
    }
    DoctorCheckResult::ok(REQUIRED_BINARIES, "Every program it runs is installed")
}

#[cfg(test)]
mod tests {
    use qol_headless::{DoctorStatus, EXIT_USAGE};

    use super::*;
    use crate::cleanup::{Amount, Item, Why};

    fn run(args: &[&str]) -> u8 {
        let args: Vec<String> = args.iter().map(ToString::to_string).collect();
        let code = app().run(args);
        if code == ExitCode::SUCCESS {
            0
        } else if code == ExitCode::from(EXIT_USAGE) {
            EXIT_USAGE
        } else {
            1
        }
    }

    #[test]
    fn help_and_usage_errors_follow_the_headless_contract() {
        assert_eq!(run(&["help"]), 0);
        assert_eq!(run(&["help", "status"]), 0);
        assert_eq!(run(&["settings", "help"]), 0);
        assert_eq!(run(&["status", "--all"]), EXIT_USAGE);
        assert_eq!(run(&["press"]), 1);
    }

    #[test]
    fn the_old_kernels_line_names_what_removes_them() {
        use crate::cleanup::fake::{self, UNATTENDED_KERNELS};
        let mut mint = fake::mint();
        assert_eq!(kernels(&mint), "removed weekly");
        mint[1].state = State::On { idle: false };
        assert_eq!(
            kernels(&mint),
            "removed weekly, kernels installed by hand too"
        );
        mint[0].state = State::Off;
        assert_eq!(kernels(&mint), "kept");
        mint[0].state = State::Unknown("the timer and its flag disagree");
        assert_eq!(kernels(&mint), "unknown, the timer and its flag disagree");
        let idle = [Live {
            switch: &UNATTENDED_KERNELS,
            state: State::On { idle: true },
        }];
        assert_eq!(kernels(&idle), "kept");
        assert_eq!(kernels(&[]), "kept, nothing removes them on its own here");
    }

    #[test]
    fn the_cleanup_listing_names_each_kind_once_and_totals_without_double_counting() {
        use crate::cleanup::fake::{self, found, LAST, OLD, ORPHANS};
        let survey = fake::survey(vec![
            found(&OLD, 1, &[("linux-image-1", 400_000_000)]),
            found(
                &ORPHANS,
                2,
                &[("linux-image-1", 400_000_000), ("libfoo", 2_000_000)],
            ),
            found(&LAST, 1, &[("core20_2015", 64_000_000)]),
        ]);
        let text = listing(&survey, &fake::mint());
        assert!(
            text.contains("kernels [kernels]: 1 old kernel, 400 MB\n"),
            "{text}"
        );
        assert!(
            text.contains("orphans [orphans]: 2 packages, 402 MB, takes kernels along\n"),
            "{text}"
        );
        assert!(
            text.contains("last [last]: 1 old version, 64 MB, asks first\n"),
            "{text}"
        );
        assert!(text.contains("Altogether: 466 MB\n"), "{text}");
        assert!(
            text.contains("Switch, mint's cleanup: Old kernels and leftovers: on\n"),
            "{text}"
        );
        let odd = fake::survey(vec![
            Found {
                leftover: &OLD,
                amount: Some(Amount {
                    count: 1,
                    present: 1,
                    size: Size::Estimate {
                        bytes: 1_500_000,
                        why: "dpkg's estimate",
                    },
                    items: vec![Item {
                        name: "linux-image-1".into(),
                        bytes: Some(1_500_000),
                    }],
                }),
            },
            Found {
                leftover: &ORPHANS,
                amount: Some(Amount::unknown(Why::NeedsAdmin)),
            },
            Found {
                leftover: &LAST,
                amount: None,
            },
        ]);
        let text = listing(&odd, &[]);
        assert!(
            text.contains("kernels [kernels]: 1 old kernel, about 1.5 MB (dpkg's estimate)\n"),
            "{text}"
        );
        assert!(
            text.contains("orphans [orphans]: size unknown, needs an administrator to measure, takes kernels along\n"),
            "{text}"
        );
        assert!(
            text.contains("last [last]: not here, it went away, asks first\n"),
            "{text}"
        );
        assert!(
            text.contains("Altogether: about 1.5 MB, and 1 kind of unknown size\n"),
            "{text}"
        );
        assert!(!text.contains("Switch,"), "{text}");
    }

    #[test]
    fn a_kind_check_only_says_whether_the_kind_can_be_measured() {
        use crate::cleanup::fake::OLD;
        let here = kind_result("cleanup_kernels", &OLD, None);
        assert_eq!(here.status, DoctorStatus::Ok);
        assert!(
            here.message.contains("cleanup` measures them"),
            "{}",
            here.message
        );
        let gone = kind_result(
            "cleanup_kernels",
            &OLD,
            Some("apt-get is not installed".into()),
        );
        assert_eq!(gone.status, DoctorStatus::Ok);
        assert_eq!(gone.message, "kernels: not here, apt-get is not installed");
    }

    #[test]
    fn remove_all_leaves_out_kinds_that_ask_and_one_kind_takes_its_consent() {
        use crate::cleanup::fake::{ALONG, BY_HAND, LAST, OLD, ORPHANS};
        static TABLE: [&Leftover; 5] = [&OLD, &BY_HAND, &ORPHANS, &LAST, &ALONG];
        let (all, consent) = choose("all", false, &TABLE).expect("all");
        let ids: Vec<&str> = all.iter().map(|kind| kind.id).collect();
        assert_eq!(ids, ["kernels", "orphans"]);
        assert_eq!(consent, Consent::All);
        assert!(choose("all", true, &TABLE).is_err());
        let (last, consent) = choose("last", false, &TABLE).expect("last");
        assert_eq!((last[0].id, consent), ("last", Consent::All));
        let (last, consent) = choose("last", true, &TABLE).expect("last --confirm");
        assert_eq!((last[0].id, consent), ("last", Consent::Asked));
        assert!(choose("nothing", false, &TABLE).is_err());
    }

    #[test]
    fn doctor_has_one_line_per_kind() {
        let ids: Vec<String> = checks()
            .iter()
            .map(|check| check.id().to_string())
            .collect();
        for kind in leftovers() {
            let id = format!("cleanup_{}", kind.id);
            assert_eq!(ids.iter().filter(|known| **known == id).count(), 1, "{id}");
        }
        assert_eq!(ids.len(), 2 + leftovers().len());
    }

    #[test]
    fn remove_takes_one_kind_or_all() {
        assert_eq!(run(&["cleanup", "remove"]), EXIT_USAGE);
        assert_eq!(run(&["cleanup", "remove", "nothing"]), EXIT_USAGE);
        assert_eq!(run(&["cleanup", "sweep"]), EXIT_USAGE);
        assert_eq!(run(&["cleanup", "remove", "all", "--confirm"]), EXIT_USAGE);
        assert_eq!(run(&["help", "cleanup"]), 0);
    }

    #[test]
    fn checks_offer_a_fix_only_when_they_fail() {
        for result in [platform_supported(), required_binaries()] {
            if result.status == DoctorStatus::Ok {
                assert!(result.fix.is_none(), "{} must not offer a fix", result.id);
            }
        }
    }
}
