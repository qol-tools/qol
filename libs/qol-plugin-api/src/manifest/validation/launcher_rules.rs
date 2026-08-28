use crate::manifest::{LauncherKind, LauncherSpec, PluginManifest};
use anyhow::{bail, Result};
use std::collections::BTreeSet;

pub(super) fn validate_optional_launcher(
    launcher: Option<&LauncherSpec>,
    executable_action_ids: &BTreeSet<String>,
) -> Result<()> {
    let Some(launcher) = launcher else {
        return Ok(());
    };

    if launcher.title.trim().is_empty() {
        bail!("launcher.title must not be empty");
    }
    if launcher
        .prompt
        .as_deref()
        .is_some_and(|prompt| prompt.trim().is_empty())
    {
        bail!("launcher.prompt must not be empty");
    }

    match launcher.kind {
        LauncherKind::Flow => {
            let Some(query) = launcher
                .query
                .as_deref()
                .map(str::trim)
                .filter(|query| !query.is_empty())
            else {
                bail!("launcher.query is required for kind = \"flow\"");
            };
            if !is_valid_query_name(query) {
                bail!("invalid launcher.query name: {query}");
            }
        }
        LauncherKind::App => {
            if launcher.query.is_some() {
                bail!("launcher.query is only valid for kind = \"flow\"");
            }
            if !launcher.row_actions.is_empty() {
                bail!("launcher.row_actions are only valid for kind = \"flow\"");
            }
        }
    }

    for row_action in &launcher.row_actions {
        if !executable_action_ids.contains(&row_action.action) {
            bail!(
                "launcher.row_actions references undeclared action: {}",
                row_action.action
            );
        }
    }

    Ok(())
}

pub fn validate_launcher_runtime(
    manifest: &PluginManifest,
    runtime: Option<&qol_config::contract::RuntimeSpec>,
) -> Result<()> {
    let Some(launcher) = manifest.launcher.as_ref() else {
        return Ok(());
    };
    if launcher.kind == LauncherKind::App {
        return Ok(());
    }

    let query = launcher.query.as_deref().unwrap_or_default();
    let Some(runtime) = runtime else {
        bail!("launcher flow query {query} requires qol-runtime.toml");
    };
    let Some(spec) = runtime.queries.get(query) else {
        bail!("launcher flow query not declared: {query}");
    };
    if spec
        .input
        .as_ref()
        .is_some_and(|input| input.contains_key("query"))
    {
        return Ok(());
    }

    bail!("launcher flow query {query} must declare a query input")
}

fn is_valid_query_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}
