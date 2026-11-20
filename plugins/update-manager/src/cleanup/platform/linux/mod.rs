use std::fs;
use std::time::{Duration, SystemTime};

use crate::cleanup::{
    Amount, Item, Leftover, Measure, Noun, Origin, Remove, Requires, Size, WayBack, Why,
};
use crate::platform::linux::command;
use crate::platform::linux::packages::{base, images_on_disk, running_kernel, versioned_abi};
use crate::platform::linux::privileged::run_long_script;
use crate::platform::Refusal;

mod parse;
mod switches;

pub(crate) use crate::platform::linux::command::found;
pub(crate) use switches::switches;

const REMOVE: Duration = Duration::from_secs(3600);
const SNAPS: &str = "/var/lib/snapd/snaps";
const ARCHIVES: &str = "/var/cache/apt/archives";
const CHANGES: [&str; 3] = ["/var/lib/dpkg/status", ARCHIVES, SNAPS];
const APT: Requires = Requires::Program("apt-get");
const SIMULATE: &[&str] = &["apt-get", "-s", "autoremove"];
const PURGE: [&str; 3] = ["apt-get", "purge", "-y"];
const DPKG: &str = "dpkg's estimate; apt's own count needs an administrator";
const EXEC_SH: &str = "exec env DEBIAN_FRONTEND=noninteractive LANG=C LC_ALL=C \"$@\"";
const UNKNOWN_RUNNING: &str =
    "the running kernel's version does not read as a packaged kernel, so none are listed";
const TAKES_MORE: &str = "apt would remove or install other packages with them, so none are listed";
const REFUSED: &str =
    "apt would remove or install other packages with them, so nothing was removed";
const SNAP_SH: &str = r#"for item in "$@"; do
  snap remove "${item%_*}" --revision="${item##*_}" || exit 3
done
exit 0
"#;

static KERNELS: Leftover = Leftover {
    id: "kernels",
    name: "Old kernels",
    what: "Kernels apt no longer needs. The running kernel and the two newest stay.",
    noun: Noun {
        one: "old kernel",
        many: "old kernels",
    },
    requires: APT,
    measure: Measure::Command {
        argv: SIMULATE,
        read: old_kernels,
    },
    remove: Remove::Packages,
    admin: true,
    way_back: WayBack::None,
    origin: Origin::System,
    along: &[],
};

static KERNELS_BY_HAND: Leftover = Leftover {
    id: "kernels_by_hand",
    name: "Kernels installed by hand",
    what: "Kernels installed by name or from Mint's Update Manager window. Nothing removes them on its own; the running kernel and the two newest stay.",
    noun: Noun {
        one: "kernel installed by hand",
        many: "kernels installed by hand",
    },
    requires: APT,
    measure: Measure::Command {
        argv: &["apt-mark", "showmanual"],
        read: kernels_by_hand,
    },
    remove: Remove::Packages,
    admin: true,
    way_back: WayBack::None,
    origin: Origin::ByHand,
    along: &[],
};

static ORPHANS: Leftover = Leftover {
    id: "orphans",
    name: "Packages nothing needs",
    what: "Packages installed for another one that has since gone. Old kernels are among them.",
    noun: Noun {
        one: "package",
        many: "packages",
    },
    requires: APT,
    measure: Measure::Command {
        argv: SIMULATE,
        read: orphans,
    },
    remove: Remove::Command(&["apt-get", "autoremove", "--purge", "-y"]),
    admin: true,
    way_back: WayBack::None,
    origin: Origin::System,
    along: &[&KERNELS],
};

static PACKAGE_CACHE: Leftover = Leftover {
    id: "pkgcache",
    name: "Downloaded packages",
    what:
        "Package files apt keeps after installing them. It downloads them again when it needs them.",
    noun: Noun {
        one: "package file",
        many: "package files",
    },
    requires: APT,
    measure: Measure::Files {
        dir: ARCHIVES,
        suffix: ".deb",
    },
    remove: Remove::Command(&["apt-get", "clean"]),
    admin: true,
    way_back: WayBack::None,
    origin: Origin::System,
    along: &[],
};

static FLATPAK: Leftover = Leftover {
    id: "flatpak",
    name: "Unused Flatpak runtimes",
    what: "Runtimes no installed app uses any more.",
    noun: Noun {
        one: "runtime",
        many: "runtimes",
    },
    requires: Requires::Program("flatpak"),
    measure: Measure::Unknown(
        "Flatpak 1.14 has no dry run for unused runtimes, and its listed sizes overcount files runtimes share",
    ),
    remove: Remove::Command(&["flatpak", "uninstall", "--unused", "-y", "--noninteractive"]),
    admin: false,
    way_back: WayBack::None,
    origin: Origin::System,
    along: &[],
};

static SNAP: Leftover = Leftover {
    id: "snap",
    name: "Old snap versions",
    what: "The version each snap ran before its last update, kept so it can go back.",
    noun: Noun {
        one: "old snap version",
        many: "old snap versions",
    },
    requires: Requires::Program("snap"),
    measure: Measure::Command {
        argv: &["snap", "list", "--all"],
        read: snaps,
    },
    remove: Remove::Script(SNAP_SH),
    admin: true,
    way_back: WayBack::Last {
        to: "the version before each snap's last update",
    },
    origin: Origin::System,
    along: &[],
};

static LEFTOVERS: [&Leftover; 6] = [
    &KERNELS,
    &KERNELS_BY_HAND,
    &ORPHANS,
    &PACKAGE_CACHE,
    &FLATPAK,
    &SNAP,
];

pub(crate) fn leftovers() -> &'static [&'static Leftover] {
    &LEFTOVERS
}

pub(crate) fn stamp() -> Vec<Option<SystemTime>> {
    CHANGES
        .iter()
        .map(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok())
        .collect()
}

pub(crate) fn output(argv: &[&str]) -> Result<String, String> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| "nothing to run".to_string())?;
    command::output(program, args)
}

pub(crate) fn run(remove: &Remove, items: &[String], admin: bool) -> Result<(), Refusal> {
    run_checked(remove, items, admin, simulate_purge)
}

fn run_checked(
    remove: &Remove,
    items: &[String],
    admin: bool,
    simulate: impl FnOnce(&[String]) -> Result<String, String>,
) -> Result<(), Refusal> {
    if *remove == Remove::Packages && !alone(items, simulate).map_err(Refusal::Failed)? {
        return Err(Refusal::Failed(REFUSED.into()));
    }
    let (script, args) = script(remove, items);
    if admin {
        return run_long_script(&script, &args, REMOVE);
    }
    let done = command::script(&script, &args, REMOVE).map_err(Refusal::Failed)?;
    if done.code == Some(0) {
        Ok(())
    } else {
        Err(Refusal::Failed(done.reason()))
    }
}

fn script(remove: &Remove, items: &[String]) -> (String, Vec<String>) {
    let argv: &[&str] = match remove {
        Remove::Command(argv) => argv,
        Remove::Packages => &PURGE,
        Remove::Script(script) => return ((*script).into(), items.to_vec()),
    };
    let mut args: Vec<String> = argv.iter().map(|arg| arg.to_string()).collect();
    if *remove == Remove::Packages {
        args.extend(items.iter().cloned());
    }
    (EXEC_SH.into(), args)
}

fn simulate_purge(packages: &[String]) -> Result<String, String> {
    let mut args = vec!["-s", "purge"];
    args.extend(packages.iter().map(String::as_str));
    command::output("apt-get", &args)
}

fn alone(
    packages: &[String],
    simulate: impl FnOnce(&[String]) -> Result<String, String>,
) -> Result<bool, String> {
    Ok(!parse::plans_beyond(&simulate(packages)?, packages))
}

fn listed(
    packages: Vec<String>,
    count: usize,
    present: usize,
    simulate: impl FnOnce(&[String]) -> Result<String, String>,
) -> Amount {
    if packages.is_empty() {
        return dpkg_sized(packages, count, present);
    }
    match alone(&packages, simulate) {
        Ok(true) => dpkg_sized(packages, count, present),
        Ok(false) => unknown(Why::Cannot(TAKES_MORE), present),
        Err(reason) => unknown(Why::Failed(reason), present),
    }
}

fn unknown(why: Why, present: usize) -> Amount {
    Amount {
        present,
        ..Amount::unknown(why)
    }
}

fn old_kernels(simulated: &str) -> Result<Amount, String> {
    let packages: Vec<String> = parse::removals(simulated)
        .into_iter()
        .filter(|package| versioned_abi(package).is_some())
        .collect();
    let count = parse::kernels(&packages);
    Ok(listed(packages, count, count, simulate_purge))
}

fn orphans(simulated: &str) -> Result<Amount, String> {
    let packages = parse::removals(simulated);
    let count = packages.len();
    Ok(dpkg_sized(packages, count, count))
}

fn kernels_by_hand(manual: &str) -> Result<Amount, String> {
    let running = running_kernel();
    let present = parse::kept(manual, running.as_deref());
    Ok(
        match parse::by_hand(manual, running.as_deref(), images_on_disk) {
            Ok(Some(by_hand)) => listed(by_hand.packages, by_hand.kernels, present, simulate_purge),
            Ok(None) => unknown(Why::Cannot(UNKNOWN_RUNNING), present),
            Err(reason) => unknown(Why::Failed(reason), present),
        },
    )
}

fn snaps(listing: &str) -> Result<Amount, String> {
    let items: Vec<Item> = parse::disabled_snaps(listing)
        .into_iter()
        .map(|name| {
            let bytes = fs::metadata(format!("{SNAPS}/{name}.snap"))
                .ok()
                .map(|meta| meta.len());
            Item { name, bytes }
        })
        .collect();
    let size = if items.iter().all(|item| item.bytes.is_some()) {
        Size::Exact(items.iter().filter_map(|item| item.bytes).sum())
    } else {
        Size::Unknown(Why::Failed(format!(
            "some files in {SNAPS} could not be read"
        )))
    };
    Ok(Amount {
        count: items.len(),
        present: items.len(),
        size,
        items,
    })
}

fn dpkg_sized(packages: Vec<String>, count: usize, present: usize) -> Amount {
    sized_by(packages, count, present, |args| {
        command::output("dpkg-query", args)
    })
}

fn sized_by(
    packages: Vec<String>,
    count: usize,
    present: usize,
    query: impl FnOnce(&[&str]) -> Result<String, String>,
) -> Amount {
    if packages.is_empty() {
        return Amount {
            count,
            present,
            size: Size::Exact(0),
            items: Vec::new(),
        };
    }
    let mut args = vec!["-W", "-f=${Package}\t${Installed-Size}\n"];
    args.extend(packages.iter().map(String::as_str));
    let sizes = match query(&args) {
        Ok(listing) => parse::sizes(&listing),
        Err(reason) => return unknown(Why::Failed(reason), present),
    };
    let items: Vec<Item> = packages
        .into_iter()
        .map(|name| {
            let bytes = sizes.get(base(&name)).copied();
            Item { name, bytes }
        })
        .collect();
    Amount {
        count,
        present,
        size: Size::Estimate {
            bytes: items.iter().filter_map(|item| item.bytes).sum(),
            why: DPKG,
        },
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linux_table_has_exactly_these_kinds() {
        let ids: Vec<&str> = leftovers().iter().map(|leftover| leftover.id).collect();
        assert_eq!(
            ids,
            [
                "kernels",
                "kernels_by_hand",
                "orphans",
                "pkgcache",
                "flatpak",
                "snap"
            ]
        );
        let asking: Vec<&str> = leftovers()
            .iter()
            .filter(|leftover| crate::cleanup::why(leftover).is_some())
            .map(|leftover| leftover.id)
            .collect();
        assert_eq!(asking, ["kernels_by_hand", "snap"]);
    }

    #[test]
    fn the_page_fakes_carry_the_real_kernel_nouns() {
        use crate::cleanup::fake;
        assert_eq!(fake::OLD.noun, KERNELS.noun);
        assert_eq!(fake::BY_HAND.noun, KERNELS_BY_HAND.noun);
    }

    const ALONE: &str = "Purg linux-image-6.8.0-111-generic [6.8.0-111.111]\n";

    fn kernel() -> Vec<String> {
        vec!["linux-image-6.8.0-111-generic".to_string()]
    }

    fn unasked(_: &[String]) -> Result<String, String> {
        panic!("nothing to simulate")
    }

    #[test]
    fn a_failed_size_query_keeps_the_count_of_kernels_kept() {
        let failed = sized_by(kernel(), 1, 3, |_| Err("dpkg-query broke".into()));
        assert_eq!(failed.present, 3);
        assert_eq!(
            failed.size,
            Size::Unknown(Why::Failed("dpkg-query broke".into()))
        );
        assert!(failed.items.is_empty());
        let sized = sized_by(kernel(), 1, 3, |args| {
            assert_eq!(args.last(), Some(&"linux-image-6.8.0-111-generic"));
            Ok("linux-image-6.8.0-111-generic\t100\n".into())
        });
        assert_eq!((sized.count, sized.present), (1, 3));
        assert_eq!(sized.items[0].bytes, Some(100 * 1024));
        let none = sized_by(Vec::new(), 0, 2, |_| panic!("nothing to query"));
        assert_eq!((none.present, none.size), (2, Size::Exact(0)));
    }

    #[test]
    fn kernels_are_listed_only_when_the_purge_plan_takes_nothing_else() {
        assert_eq!(alone(&kernel(), |_| Ok(ALONE.into())), Ok(true));
        assert_eq!(
            alone(&kernel(), |_| Ok(format!(
                "{ALONE}Purg linux-generic [6.8.0.111]\n"
            ))),
            Ok(false)
        );
        assert_eq!(
            alone(&kernel(), |_| Err("apt is locked".into())),
            Err("apt is locked".to_string())
        );
        let more = listed(kernel(), 1, 3, |_| {
            Ok(format!(
                "Inst linux-image-unsigned-6.8.0-111-generic\n{ALONE}"
            ))
        });
        assert_eq!(more, unknown(Why::Cannot(TAKES_MORE), 3));
        assert!(more.items.is_empty(), "nothing is listed for removal");
        let failed = listed(kernel(), 1, 3, |_| Err("apt is locked".into()));
        assert_eq!(failed, unknown(Why::Failed("apt is locked".into()), 3));
        let none = listed(Vec::new(), 0, 3, unasked);
        assert_eq!((none.size, none.present), (Size::Exact(0), 3));
    }

    #[test]
    fn a_purge_that_would_take_more_is_refused_before_anything_runs() {
        let absent = vec!["qol-cleanup-test-no-such-package".to_string()];
        let refused = run_checked(&Remove::Packages, &absent, false, |_| {
            Ok("Purg qol-cleanup-test-no-such-package\nPurg linux-generic\n".into())
        });
        assert_eq!(refused, Err(Refusal::Failed(REFUSED.into())));
        let unread = run_checked(&Remove::Packages, &absent, false, |_| {
            Err("apt is locked".into())
        });
        assert_eq!(unread, Err(Refusal::Failed("apt is locked".into())));
    }

    #[test]
    fn a_removal_runs_its_command_with_the_items_it_takes() {
        let items = vec!["linux-image-6.8.0-111-generic".to_string()];
        let (exec, args) = script(&KERNELS.remove, &items);
        assert_eq!(exec, EXEC_SH);
        assert_eq!(script(&KERNELS_BY_HAND.remove, &items).1, args);
        assert_eq!(
            args,
            ["apt-get", "purge", "-y", "linux-image-6.8.0-111-generic"]
        );
        let (_, args) = script(&ORPHANS.remove, &items);
        assert_eq!(args, ["apt-get", "autoremove", "--purge", "-y"]);
        let (snap, args) = script(&SNAP.remove, &["core20_2015".to_string()]);
        assert!(snap.contains("--revision=\"${item##*_}\""));
        assert_eq!(args, ["core20_2015"]);
    }
}
