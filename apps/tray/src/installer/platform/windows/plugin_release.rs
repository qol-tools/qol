use anyhow::{ensure, Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::plugins::registry::{load_registry, registry_path};
use crate::plugins::resolver::{resolve_effective_registry, ResolvedPlugin};
use crate::plugins::PluginLoader;

const RELEASE_COMMAND: &str = "release-host";
const RELEASE_TIMEOUT: Duration = Duration::from_secs(150);

pub(super) fn release_all() -> Result<()> {
    let config_dir = crate::paths::shared_config_dir()?;
    if !registry_path(&config_dir).try_exists()? {
        return Ok(());
    }
    let registry = load_registry(&config_dir).map_err(anyhow::Error::msg)?;
    let failures = resolve_effective_registry(&registry, &config_dir)
        .plugins
        .iter()
        .filter_map(|resolved| {
            release(resolved)
                .err()
                .map(|error| format!("{}: {error:#}", resolved.id))
        })
        .collect::<Vec<_>>();
    ensure!(failures.is_empty(), "{}", failures.join("; "));
    Ok(())
}

fn release(resolved: &ResolvedPlugin) -> Result<()> {
    let Some(plugin) = PluginLoader::load_resolved_plugin(resolved)? else {
        return Ok(());
    };
    if !plugin.manifest.capabilities.release_host {
        return Ok(());
    }
    let runtime = plugin
        .manifest
        .runtime
        .as_ref()
        .context("declares release_host without a runtime command")?;
    let program = crate::plugins::resolve_plugin_command_path_for_source(
        &plugin.path,
        &runtime.command,
        Some(&plugin.source),
    )
    .with_context(|| format!("could not find its runtime command {:?}", runtime.command))?;
    run_release(&program, &plugin.path, plugin.id.as_str())
}

fn run_release(program: &Path, plugin_dir: &Path, plugin_id: &str) -> Result<()> {
    let mut child = qol_process::hide_console_window(
        Command::new(program)
            .arg(RELEASE_COMMAND)
            .current_dir(plugin_dir)
            .env(qol_conventions::ENV_PLUGIN_ID, plugin_id)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    )
    .spawn()
    .with_context(|| format!("could not start {}", program.display()))?;
    let status = qol_process::wait_for_exit_or_terminate(&mut child, RELEASE_TIMEOUT)
        .with_context(|| format!("{RELEASE_COMMAND} did not finish"))?;
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr).ok();
    }
    ensure!(
        status.success(),
        "{RELEASE_COMMAND} failed ({status}): {}",
        stderr.trim()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(script: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("plugin.cmd");
        std::fs::write(&program, script).unwrap();
        (dir, program)
    }

    #[test]
    fn a_release_reports_the_plugin_reason_when_it_fails() {
        let (dir, program) = cmd("@echo off\r\necho kept the rule 1>&2\r\nexit /b 3\r\n");
        let error = run_release(&program, dir.path(), "qol-test").unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("kept the rule"), "{message}");
        assert!(message.contains(RELEASE_COMMAND), "{message}");
    }

    #[test]
    fn a_release_passes_its_command_and_plugin_id() {
        let (dir, program) = cmd(&format!(
            "@echo off\r\nif not \"%1\"==\"{RELEASE_COMMAND}\" exit /b 4\r\nif not \"%{}%\"==\"qol-test\" exit /b 5\r\nexit /b 0\r\n",
            qol_conventions::ENV_PLUGIN_ID
        ));
        run_release(&program, dir.path(), "qol-test").unwrap();
    }
}
