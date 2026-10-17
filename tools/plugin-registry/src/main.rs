#![allow(clippy::print_stderr)]

mod artifact;
mod backfill;
mod deployed;
mod git;
mod host;
mod index;
mod oci;
mod parallel;
mod provenance;
mod prune;
mod registry;
mod release;

use anyhow::{Context, Result};
use artifact::Pushed;
use host::HostConfig;
use provenance::{GhAttestation, Provenance};
use qol_plugin_index::HOST_KEEP;
use registry::{Access, Credentials, Registry};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const USAGE: &str = "\
usage: qol-plugin-registry <command> [options]

  push        --registry R --source URL --tag T --files DIR [--revision REV]
  push-host   --registry R --source URL --os OS --version V --revision SHA --files DIR
  backfill    --registry R --source URL --github-repo OWNER/REPO
  index       --registry R --public-key FILE --output FILE [--previous-url URL] [--keep N]
              [--provenance-repo OWNER/REPO --provenance-workflow PATH]
  push-index  --registry R --signed FILE
  prune       --registry R [--keep-host N] [--dry-run]

The registry is ghcr.io/OWNER/plugins or http://HOST:PORT/OWNER/plugins.
REGISTRY_USERNAME and REGISTRY_PASSWORD log in to the registry and GH_TOKEN
to the GitHub API and the gh CLI. The index is signed separately, with the
minisign CLI.";

fn main() {
    if let Err(error) = run(pico_args::Arguments::from_env()) {
        eprintln!("::error::{error:#}");
        std::process::exit(1);
    }
}

fn run(mut args: pico_args::Arguments) -> Result<()> {
    let command = args.subcommand()?.unwrap_or_default();
    match command.as_str() {
        "push" => push(&mut args)?,
        "push-host" => push_host(&mut args)?,
        "backfill" => backfill(&mut args)?,
        "index" => index(&mut args)?,
        "push-index" => push_index(&mut args)?,
        "prune" => prune(&mut args)?,
        _ => anyhow::bail!("{USAGE}"),
    }
    let unused = args.finish();
    if !unused.is_empty() {
        anyhow::bail!("unexpected arguments {unused:?}\n{USAGE}");
    }
    Ok(())
}

fn push(args: &mut pico_args::Arguments) -> Result<()> {
    let registry = registry(args, Access::PullPush)?;
    let source: String = args.value_from_str("--source")?;
    let tag: String = args.value_from_str("--tag")?;
    let files: PathBuf = args.value_from_str("--files")?;
    let revision = args
        .opt_value_from_str("--revision")?
        .unwrap_or_else(|| "HEAD".to_string());
    let release = artifact::release_from_git(&tag, &revision, artifact::read_assets(&files)?)?;
    let outcome = artifact::push(&registry, &release, &source)?;
    say(&format!("{tag}: {}", pushed_label(&outcome)))?;
    say(&format!("registry requests: {}", registry.requests()))
}

fn push_host(args: &mut pico_args::Arguments) -> Result<()> {
    let registry = registry(args, Access::PullPush)?;
    let source: String = args.value_from_str("--source")?;
    let config = HostConfig {
        os: args.value_from_str("--os")?,
        version: args.value_from_str("--version")?,
        revision: args.value_from_str("--revision")?,
    };
    let files: PathBuf = args.value_from_str("--files")?;
    let build = host::host_build(config, artifact::read_assets(&files)?)?;
    let outcome = host::push(&registry, &build, &source)?;
    say(&format!("{}: {}", build.tag, pushed_label(&outcome)))?;
    say(&format!("registry requests: {}", registry.requests()))
}

fn backfill(args: &mut pico_args::Arguments) -> Result<()> {
    let registry = registry(args, Access::PullPush)?;
    let source: String = args.value_from_str("--source")?;
    let github_repo: String = args.value_from_str("--github-repo")?;
    let token = std::env::var("GH_TOKEN").ok();
    let outcomes = backfill::backfill(
        &registry,
        &registry::agent(),
        &github_repo,
        token.as_deref(),
        &source,
    )?;
    for outcome in &outcomes {
        let label = outcome
            .pushed
            .as_ref()
            .map_or("skipped, already in the registry", pushed_label);
        say(&format!("{}: {label}", outcome.tag))?;
    }
    say(&format!("registry requests: {}", registry.requests()))
}

fn index(args: &mut pico_args::Arguments) -> Result<()> {
    let registry = registry(args, Access::Pull)?;
    let public_key = public_key(args)?;
    let output: PathBuf = args.value_from_str("--output")?;
    let previous_url: Option<String> = args.opt_value_from_str("--previous-url")?;
    let keep: usize = args.opt_value_from_str("--keep")?.unwrap_or(3);
    if keep == 0 {
        anyhow::bail!("--keep must be at least 1");
    }
    let provenance = match (
        args.opt_value_from_str("--provenance-repo")?,
        args.opt_value_from_str("--provenance-workflow")?,
    ) {
        (Some(repo), Some(workflow)) => Some(GhAttestation { repo, workflow }),
        (None, None) => None,
        _ => anyhow::bail!("--provenance-repo and --provenance-workflow go together"),
    };
    let previous = match &previous_url {
        Some(url) => deployed::fetch(&registry::agent(), url, &public_key)?,
        None => None,
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let built = index::build(
        &registry,
        previous.as_ref(),
        keep,
        now,
        provenance
            .as_ref()
            .map(|verifier| verifier as &dyn Provenance),
    )?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, serde_json::to_vec(&built.document)?)
        .with_context(|| format!("cannot write {}", output.display()))?;
    for (plugin_id, plugin) in &built.document.plugins {
        say(&format!(
            "{plugin_id} {} ({} versions)",
            plugin.latest,
            plugin.versions.len()
        ))?;
    }
    for (os, host) in &built.document.host {
        say(&format!(
            "host {os} {} ({} versions)",
            host.latest,
            host.versions.len()
        ))?;
    }
    for change in index::changes(previous.as_ref(), &built.document) {
        say(&change)?;
    }
    for warning in &built.warnings {
        say(&format!("::warning::{warning}"))?;
    }
    say(&format!(
        "serial {}: {} versions reused from the deployed index, {} read from the registry, {} registry requests",
        built.document.serial,
        built.reused,
        built.fetched,
        registry.requests()
    ))
}

fn push_index(args: &mut pico_args::Arguments) -> Result<()> {
    let registry = registry(args, Access::PullPush)?;
    let path: PathBuf = args.value_from_str("--signed")?;
    let signed = std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let serial = deployed::push(&registry, &signed)?;
    say(&format!("{}: serial {serial}", qol_plugin_index::INDEX_TAG))?;
    say(&format!("registry requests: {}", registry.requests()))
}

fn prune(args: &mut pico_args::Arguments) -> Result<()> {
    let reference: String = args.value_from_str("--registry")?;
    let keep_host: usize = args.opt_value_from_str("--keep-host")?.unwrap_or(HOST_KEEP);
    if keep_host == 0 {
        anyhow::bail!("--keep-host must be at least 1");
    }
    let dry_run = args.contains("--dry-run");
    let token = std::env::var("GH_TOKEN").context("prune needs GH_TOKEN")?;
    let packages = prune::Packages::new(&registry::location(&reference)?, token)?;
    let versions = packages.versions()?;
    let pull = Registry::new(&reference, Access::Pull, None)?;
    let doomed = prune::doomed(&versions, keep_host, |version| {
        prune::is_index_copy(&pull, version)
    });
    let verb = if dry_run { "would delete" } else { "deleted" };
    for version in &doomed {
        if !dry_run {
            packages.delete(version)?;
        }
        say(&format!("{verb} {}", prune::describe(version)))?;
    }
    say(&format!(
        "{verb} {} of {} package versions",
        doomed.len(),
        versions.len()
    ))
}

fn registry(args: &mut pico_args::Arguments, access: Access) -> Result<Registry> {
    let reference: String = args.value_from_str("--registry")?;
    let credentials = match (
        std::env::var("REGISTRY_USERNAME"),
        std::env::var("REGISTRY_PASSWORD"),
    ) {
        (Ok(username), Ok(password)) => Some(Credentials { username, password }),
        _ => None,
    };
    Registry::new(&reference, access, credentials)
}

fn public_key(args: &mut pico_args::Arguments) -> Result<String> {
    let path: PathBuf = args.value_from_str("--public-key")?;
    let file = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    Ok(qol_plugin_index::public_key_line(&file).to_string())
}

fn pushed_label(pushed: &Pushed) -> &'static str {
    match pushed {
        Pushed::New => "pushed",
        Pushed::AlreadyThere => "already in the registry with the same content",
    }
}

fn say(line: &str) -> Result<()> {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    writeln!(stdout, "{line}")?;
    Ok(())
}
