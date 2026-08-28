use std::ffi::OsString;

use anyhow::{anyhow, bail, Context, Result};
use qol_agent_homes::{Harness, Registry};
use qol_conventions::{local_base_url, HTTP_AGENT_HOME_HEADER, HTTP_AUTH_HEADER};

mod configure;

use configure::{apply_codex_entry, apply_json_entry, codex_entry_text, config_path, json_entry};

const USAGE: &str = "qol mcp <url|token|headers|configure>";

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    match args.first().and_then(|value| value.to_str()) {
        None | Some("help" | "-h" | "--help") => {
            print!("{}", help_text());
            Ok(())
        }
        Some("url") => {
            println!("{}", url_text());
            Ok(())
        }
        Some("token") => {
            println!("{}", token_text()?);
            Ok(())
        }
        Some("headers") => {
            let token = token_text()?;
            println!("{}", headers_text(&token)?);
            Ok(())
        }
        Some("configure") => run_configure(args.get(1..).unwrap_or_default()),
        Some(other) => bail!("unknown qol mcp subcommand `{other}`\n\n{}", help_text()),
    }
}

fn run_configure(args: &[OsString]) -> Result<()> {
    let name = args
        .first()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("usage: {USAGE}"))?;
    let harness =
        Harness::parse(name).ok_or_else(|| anyhow!("unknown harness `{name}`; usage: {USAGE}"))?;
    let path = config_path(harness)?;
    let document = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "config file not found at {}; create it before running qol mcp configure",
            path.display()
        )
    })?;
    let url = url_text();
    let token = token_text()?;
    let (updated, entry_text, baked) = match harness {
        Harness::Codex => {
            let agent_home = Registry::load().current(Harness::Codex).id;
            let updated = apply_codex_entry(&document, &url, &token, &agent_home)?;
            let entry = codex_entry_text(&url, &token, &agent_home)?;
            (updated, entry, Some(agent_home))
        }
        Harness::Kimi => {
            let agent_home = Registry::load().current(Harness::Kimi).id;
            let entry = json_entry(Harness::Kimi, &url, &token, &agent_home);
            let text = serde_json::to_string_pretty(&entry)
                .map_err(|error| anyhow!("failed to render the mcp entry: {error}"))?;
            (apply_json_entry(&document, entry)?, text, Some(agent_home))
        }
        other => {
            let agent_home = Registry::load().current(other).id;
            let entry = json_entry(other, &url, &token, &agent_home);
            let text = serde_json::to_string_pretty(&entry)
                .map_err(|error| anyhow!("failed to render the mcp entry: {error}"))?;
            (apply_json_entry(&document, entry)?, text, None)
        }
    };
    std::fs::write(&path, updated)
        .with_context(|| format!("failed to write {}", path.display()))?;
    println!("updated {}", path.display());
    println!("{entry_text}");
    if let Some(agent_home) = baked {
        println!("agent home {agent_home} baked in; run qol mcp configure again after re-homing");
    }
    Ok(())
}

fn url_text() -> String {
    format!("{}/api/mcp", local_base_url())
}

fn token_text() -> Result<String> {
    let path = qol_config::http_auth_token_path()
        .ok_or_else(|| anyhow!("qol-tray HTTP token not found; start qol-tray first"))?;
    std::fs::read_to_string(&path)
        .map(|token| token.trim().to_owned())
        .map_err(|_| {
            anyhow!(
                "qol-tray HTTP token not found at {}; start qol-tray first",
                path.display()
            )
        })
}

fn headers_text(token: &str) -> Result<String> {
    let mut headers = serde_json::Map::new();
    headers.insert(
        HTTP_AUTH_HEADER.to_owned(),
        serde_json::Value::String(token.to_owned()),
    );
    headers.insert(
        HTTP_AGENT_HOME_HEADER.to_owned(),
        serde_json::Value::String(Registry::load().current(Harness::Claude).id),
    );
    serde_json::to_string(&serde_json::Value::Object(headers))
        .map_err(|error| anyhow!("failed to serialize the headers object: {error}"))
}

fn help_text() -> &'static str {
    r#"qol mcp

Print the connection facts for the qol-tray MCP endpoint and configure agent harnesses.

Usage:
  qol mcp url
  qol mcp token
  qol mcp headers
  qol mcp configure <claude|codex|pi|kimi>
  qol mcp help

Details:
  url prints the local streamable HTTP endpoint URL served by qol-tray.
  token prints the tray HTTP auth token from the qol config directory.
  headers prints a compact JSON object mapping the auth header to the token and
  the x-qol-agent-home header to the agent home id.
  configure writes or replaces the qol MCP entry in the named harness's user
  config; claude and pi reference the qol CLI for their headers and token,
  while codex and kimi embed the token value directly. Existing servers,
  tables, and comments survive; the config file must already exist.
"#
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_conventions::{DEFAULT_PORT, LOCAL_HOST};

    #[test]
    fn url_text_is_built_from_the_host_constants() {
        assert_eq!(
            url_text(),
            format!("http://{LOCAL_HOST}:{DEFAULT_PORT}/api/mcp")
        );
    }
}
