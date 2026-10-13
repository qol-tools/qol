use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use windows::core::HSTRING;
use windows::ApplicationModel::{Package, PackageSignatureKind};
use windows::Management::Deployment::PackageManager;

use crate::cli::PLUGIN_ID;

const CACHE_TTL: Duration = Duration::from_secs(10);
const RESOURCE_PREFIX: &str = "ms-resource:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StorePackage {
    pub(super) full_name: String,
    pub(super) family_name: String,
    pub(super) name: String,
    pub(super) install_dir: PathBuf,
    pub(super) protected: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Kind {
    pub(super) framework: bool,
    pub(super) resource: bool,
    pub(super) bundle: bool,
    pub(super) system: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Listing {
    Hidden,
    Listed { protected: bool },
}

pub(super) fn listing(kind: Kind) -> Listing {
    if kind.resource || kind.bundle || kind.framework {
        return Listing::Hidden;
    }
    Listing::Listed {
        protected: kind.system,
    }
}

pub(super) fn display_name(display: &str, package_name: &str) -> String {
    let display = display.trim();
    if display.is_empty() || display.starts_with(RESOURCE_PREFIX) {
        package_name.to_string()
    } else {
        display.to_string()
    }
}

static CACHE: Mutex<Option<(Instant, Arc<Vec<StorePackage>>)>> = Mutex::new(None);

pub(super) fn packages() -> Arc<Vec<StorePackage>> {
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((read_at, packages)) = cache.as_ref() {
        if read_at.elapsed() < CACHE_TTL {
            return Arc::clone(packages);
        }
    }
    let packages = Arc::new(read_packages().unwrap_or_else(|error| {
        log::warn!("[{PLUGIN_ID}] cannot list Microsoft Store packages: {error}");
        Vec::new()
    }));
    *cache = Some((Instant::now(), Arc::clone(&packages)));
    packages
}

pub(super) fn forget() {
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cache = None;
}

pub(super) fn remove(full_name: &str) -> Result<()> {
    let manager = PackageManager::new().context("cannot open the Windows package manager")?;
    let outcome = manager
        .RemovePackageAsync(&HSTRING::from(full_name))
        .and_then(|operation| operation.get());
    forget();
    let result =
        outcome.with_context(|| format!("{PLUGIN_ID}: Windows could not remove {full_name}"))?;
    let code = result.ExtendedErrorCode().unwrap_or_default();
    if code.is_err() {
        let text = result
            .ErrorText()
            .map(|text| text.to_string())
            .unwrap_or_default();
        bail!("{PLUGIN_ID}: Windows could not remove {full_name}: {code} {text}");
    }
    Ok(())
}

pub(super) fn is_installed(full_name: &str) -> bool {
    read_packages()
        .map(|packages| {
            packages
                .iter()
                .any(|package| package.full_name == full_name)
        })
        .unwrap_or(false)
}

fn read_packages() -> Result<Vec<StorePackage>> {
    let manager = PackageManager::new().context("cannot open the Windows package manager")?;
    let found = manager
        .FindPackagesByUserSecurityId(&HSTRING::new())
        .context("cannot enumerate packages for the current user")?;
    Ok(found
        .into_iter()
        .filter_map(|package| store_package(&package))
        .collect())
}

fn store_package(package: &Package) -> Option<StorePackage> {
    let kind = Kind {
        framework: package.IsFramework().unwrap_or(true),
        resource: package.IsResourcePackage().unwrap_or(true),
        bundle: package.IsBundle().unwrap_or(false),
        system: package
            .SignatureKind()
            .is_ok_and(|kind| kind == PackageSignatureKind::System),
    };
    let Listing::Listed { protected } = listing(kind) else {
        return None;
    };
    let id = package.Id().ok()?;
    let install_dir = PathBuf::from(package.InstalledPath().ok()?.to_string());
    if install_dir.as_os_str().is_empty() {
        return None;
    }
    let package_name = id.Name().ok()?.to_string();
    let display = package
        .DisplayName()
        .map(|name| name.to_string())
        .unwrap_or_default();
    Some(StorePackage {
        full_name: id.FullName().ok()?.to_string(),
        family_name: id.FamilyName().ok()?.to_string(),
        name: display_name(&display, &package_name),
        install_dir,
        protected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_removable_app_packages_are_listed_and_system_ones_are_protected() {
        let app = Kind::default();
        let cases = [
            ("store app", app, Listing::Listed { protected: false }),
            (
                "inbox system app",
                Kind {
                    system: true,
                    ..app
                },
                Listing::Listed { protected: true },
            ),
            (
                "framework",
                Kind {
                    framework: true,
                    ..app
                },
                Listing::Hidden,
            ),
            (
                "resource pack",
                Kind {
                    resource: true,
                    ..app
                },
                Listing::Hidden,
            ),
            (
                "bundle",
                Kind {
                    bundle: true,
                    ..app
                },
                Listing::Hidden,
            ),
        ];
        for (label, kind, expected) in cases {
            assert_eq!(listing(kind), expected, "{label}");
        }
    }

    #[test]
    fn unresolved_display_names_fall_back_to_the_package_name() {
        let cases = [
            ("Calculator", "Microsoft.WindowsCalculator", "Calculator"),
            (
                "",
                "Microsoft.WindowsCalculator",
                "Microsoft.WindowsCalculator",
            ),
            (
                "ms-resource:AppName",
                "Microsoft.WindowsCalculator",
                "Microsoft.WindowsCalculator",
            ),
            ("  Paint  ", "Microsoft.Paint", "Paint"),
        ];
        for (display, package, expected) in cases {
            assert_eq!(display_name(display, package), expected, "{display:?}");
        }
    }
}
