use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) struct Selection {
    pub(super) package: String,
    pub(super) binary: Option<String>,
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    manifest_path: PathBuf,
    default_run: Option<String>,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

pub(super) fn resolve(root: &Path, name: &str) -> Result<Selection> {
    let metadata = metadata(root)?;
    let candidates = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .map(|package| (package, package.aliases()))
        .collect::<Vec<_>>();
    let exact = candidates
        .iter()
        .filter(|(_, aliases)| aliases.iter().any(|alias| alias == name))
        .collect::<Vec<_>>();
    let shorthand = exact.is_empty();
    let matches = if shorthand {
        candidates
            .iter()
            .filter(|(_, aliases)| aliases.iter().any(|alias| is_shorthand(alias, name)))
            .collect::<Vec<_>>()
    } else {
        exact
    };
    match matches.as_slice() {
        [] => bail!("no workspace package or binary matching `{name}`"),
        [(package, _)] => Ok(Selection {
            package: package.name.clone(),
            binary: package.binary(name, shorthand)?,
        }),
        many => {
            let packages = many
                .iter()
                .map(|(package, _)| package.name.as_str())
                .collect::<Vec<_>>();
            bail!("ambiguous build target `{name}`: {}", packages.join(", "))
        }
    }
}

fn is_shorthand(alias: &str, name: &str) -> bool {
    alias.strip_prefix("plugin-") == Some(name) || alias.strip_prefix("qol-") == Some(name)
}

fn metadata(root: &Path) -> Result<Metadata> {
    let output = Command::new("cargo")
        .current_dir(root)
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--no-deps",
            "--format-version",
            "1",
        ])
        .output()
        .context("failed to inspect Cargo workspace targets")?;
    if !output.status.success() {
        bail!(
            "failed to inspect Cargo workspace targets: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).context("invalid Cargo workspace metadata")
}

impl Package {
    fn aliases(&self) -> Vec<String> {
        let mut aliases = vec![self.name.clone()];
        aliases.extend(self.binaries().map(|target| target.name.clone()));
        if let Some(directory) = self.manifest_path.parent() {
            if let Some(name) = directory.file_name().and_then(|name| name.to_str()) {
                aliases.push(name.to_string());
            }
            if let Some(plugin) = qol_workspace::read_plugin_source(directory) {
                aliases.push(plugin.id);
            }
        }
        aliases
    }

    fn binaries(&self) -> impl Iterator<Item = &Target> {
        self.targets
            .iter()
            .filter(|target| target.kind.iter().any(|kind| kind == "bin"))
    }

    fn binary(&self, name: &str, shorthand: bool) -> Result<Option<String>> {
        let binaries = self.binaries().collect::<Vec<_>>();
        let named = binaries
            .iter()
            .filter(|target| target.name == name || (shorthand && is_shorthand(&target.name, name)))
            .collect::<Vec<_>>();
        if named.len() > 1 {
            bail!("ambiguous binary `{name}` in package `{}`", self.name);
        }
        Ok(named
            .first()
            .copied()
            .or_else(|| {
                binaries
                    .iter()
                    .find(|target| Some(&target.name) == self.default_run.as_ref())
            })
            .or_else(|| binaries.iter().find(|target| target.name == self.name))
            .or_else(|| (binaries.len() == 1).then(|| &binaries[0]))
            .map(|target| target.name.clone()))
    }
}
