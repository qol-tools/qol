use std::fs;
use std::time::Duration;

use crate::cleanup::{
    Amount, Item, Leftover, Measure, Noun, Origin, Remove, Requires, Size, WayBack, Why,
};
use crate::platform::linux::command::{self, run as run_command};
use crate::platform::linux::packages::{base, kernel_abi};
use crate::platform::linux::privileged::run_script;
use crate::updates::Refusal;

mod parse;
mod switches;

pub(crate) use switches::switches;

const REMOVE: Duration = Duration::from_secs(3600);
const LABEL: &str = "qol-update-manager";
const OSRELEASE: &str = "/proc/sys/kernel/osrelease";
const SNAPS: &str = "/var/lib/snapd/snaps";
const APT: Requires = Requires::Program("apt-get");
const SIMULATE: &[&str] = &["apt-get", "-s", "autoremove"];
const PURGE: &[&str] = &["apt-get", "purge", "-y"];
const DPKG: &str = "dpkg's estimate; apt's own count needs an administrator";
const EXEC_SH: &str = "exec \"$@\"";
const UNKNOWN_RUNNING: &str =
    "the running kernel's version does not read as a packaged kernel, so none are listed";
const SNAP_SH: &str = r#"for item in "$@"; do
  snap remove "${item%_*}" --revision="${item##*_}" || exit 3
done
exit 0
"#;

static KERNELS: Leftover = Leftover {
    id: "kernels",
    name: "Old kernels",
    what: "Kernels apt no longer needs. The running kernel and the newest stay.",
    noun: Noun {
        one: "old kernel",
        many: "old kernels",
    },
    requires: APT,
    measure: Measure::Command {
        argv: SIMULATE,
        read: old_kernels,
    },
    remove: Remove::Command {
        argv: PURGE,
        items: true,
    },
    admin: true,
    way_back: WayBack::Keeps,
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
    remove: Remove::Command {
        argv: PURGE,
        items: true,
    },
    admin: true,
    way_back: WayBack::Keeps,
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
    remove: Remove::Command {
        argv: &["apt-get", "autoremove", "--purge", "-y"],
        items: false,
    },
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
        dirs: &["/var/cache/apt/archives"],
        suffix: ".deb",
    },
    remove: Remove::Command {
        argv: &["apt-get", "clean"],
        items: false,
    },
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
    remove: Remove::Command {
        argv: &["flatpak", "uninstall", "--unused", "-y", "--noninteractive"],
        items: false,
    },
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

pub(crate) fn found(program: &str) -> bool {
    command::found(program)
}

pub(crate) fn output(argv: &[&str]) -> Result<String, String> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| "nothing to run".to_string())?;
    command::output(program, args)
}

pub(crate) fn run(remove: &Remove, items: &[String], admin: bool) -> Result<(), Refusal> {
    let (script, args) = script(remove, items);
    if admin {
        return run_script(&script, &args);
    }
    let mut all = vec!["-c", script.as_str(), LABEL];
    all.extend(args.iter().map(String::as_str));
    let done = run_command("/bin/sh", &all, REMOVE).map_err(Refusal::Failed)?;
    if done.code == Some(0) {
        Ok(())
    } else {
        Err(Refusal::Failed(done.reason()))
    }
}

fn script(remove: &Remove, items: &[String]) -> (String, Vec<String>) {
    match remove {
        Remove::Command { argv, items: pass } => {
            let mut args: Vec<String> = argv.iter().map(|arg| arg.to_string()).collect();
            if *pass {
                args.extend(items.iter().cloned());
            }
            (EXEC_SH.into(), args)
        }
        Remove::Script(script) => ((*script).into(), items.to_vec()),
    }
}

fn running() -> Option<String> {
    let release = fs::read_to_string(OSRELEASE).ok()?;
    kernel_abi(&format!("linux-image-{}", release.trim()))
}

fn old_kernels(simulated: &str) -> Result<Amount, String> {
    let packages: Vec<String> = parse::removals(simulated)
        .into_iter()
        .filter(|package| parse::versioned_abi(package).is_some())
        .collect();
    let count = parse::kernels(&packages);
    Ok(dpkg_sized(packages, count))
}

fn orphans(simulated: &str) -> Result<Amount, String> {
    let packages = parse::removals(simulated);
    let count = packages.len();
    Ok(dpkg_sized(packages, count))
}

fn kernels_by_hand(manual: &str) -> Result<Amount, String> {
    if !manual
        .lines()
        .any(|package| parse::versioned_abi(package.trim()).is_some())
    {
        return Ok(dpkg_sized(Vec::new(), 0));
    }
    let Some(running) = running() else {
        return Ok(Amount::unknown(Why::Cannot(UNKNOWN_RUNNING)));
    };
    let listing = command::lenient(
        "dpkg-query",
        &[
            "-W",
            "-f=${db:Status-Status}\t${Package}\n",
            "linux-image-*",
        ],
    );
    let (packages, count) = parse::by_hand(manual, &running, &parse::installed(&listing));
    Ok(dpkg_sized(packages, count))
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
        size,
        items,
    })
}

fn dpkg_sized(packages: Vec<String>, count: usize) -> Amount {
    if packages.is_empty() {
        return Amount {
            count,
            size: Size::Exact(0),
            items: Vec::new(),
        };
    }
    let mut args = vec!["-W", "-f=${Package}\t${Installed-Size}\n"];
    args.extend(packages.iter().map(String::as_str));
    let sizes = match command::output("dpkg-query", &args) {
        Ok(listing) => parse::sizes(&listing),
        Err(reason) => return Amount::unknown(Why::Failed(reason)),
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
    }

    #[test]
    fn a_removal_runs_its_command_with_the_items_it_takes() {
        let items = vec!["linux-image-6.8.0-111-generic".to_string()];
        let (exec, args) = script(&KERNELS.remove, &items);
        assert_eq!(exec, EXEC_SH);
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
