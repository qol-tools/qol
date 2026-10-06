#![allow(clippy::print_stderr)]

mod artifact;
mod backfill;
mod deployed;
mod git;
mod index;
mod oci;
mod parallel;
mod registry;
mod release;

use anyhow::{Context, Result};
use artifact::Pushed;
use registry::{Access, Credentials, Registry};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const USAGE: &str = "\
usage: qol-plugin-registry <command> [options]

  push      --registry R --source URL --tag T --files DIR [--revision REV]
  backfill  --registry R --source URL --github-repo OWNER/REPO
  index     --registry R --public-key FILE --output FILE [--previous-url URL] [--keep N]

The registry is ghcr.io/OWNER/plugins or http://HOST:PORT/OWNER/plugins.
REGISTRY_USERNAME and REGISTRY_PASSWORD log in to the registry and GH_TOKEN
to the GitHub API. The index is signed separately, with the minisign CLI.";

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
        "backfill" => backfill(&mut args)?,
        "index" => index(&mut args)?,
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
    let previous = match &previous_url {
        Some(url) => deployed::fetch(&registry::agent(), url, &public_key)?,
        None => None,
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let built = index::build(&registry, previous.as_ref(), keep, now)?;
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
    for change in index::changes(previous.as_ref(), &built.document) {
        say(&change)?;
    }
    say(&format!(
        "serial {}: {} versions reused from the deployed index, {} read from the registry, {} registry requests",
        built.document.serial,
        built.reused,
        built.fetched,
        registry.requests()
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
