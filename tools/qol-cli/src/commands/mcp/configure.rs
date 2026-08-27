use std::path::PathBuf;

use anyhow::{anyhow, Result};
use qol_conventions::HTTP_AUTH_HEADER;

pub(crate) enum Harness {
    Claude,
    Codex,
    Pi,
    Kimi,
}

impl Harness {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "pi" => Some(Self::Pi),
            "kimi" => Some(Self::Kimi),
            _ => None,
        }
    }

    pub(crate) fn config_path(&self) -> Result<PathBuf> {
        match self {
            Self::Claude => Ok(home()?.join(".claude.json")),
            Self::Codex => Ok(home()?.join(".codex").join("config.toml")),
            Self::Pi => Ok(home()?.join(".pi").join("agent").join("mcp.json")),
            Self::Kimi => Ok(match std::env::var_os("KIMI_CODE_HOME") {
                Some(dir) => PathBuf::from(dir).join("mcp.json"),
                None => home()?.join(".kimi-code").join("mcp.json"),
            }),
        }
    }
}

fn home() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow!("cannot determine the home directory"))
}

fn headers_object(value: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut headers = serde_json::Map::new();
    headers.insert(
        HTTP_AUTH_HEADER.to_owned(),
        serde_json::Value::String(value.to_owned()),
    );
    headers
}

pub(crate) fn json_entry(harness: Harness, url: &str, token: &str) -> serde_json::Value {
    match harness {
        Harness::Claude => serde_json::json!({
            "type": "http",
            "url": url,
            "headersHelper": "qol mcp headers",
        }),
        Harness::Codex => serde_json::json!({
            "url": url,
            "http_headers": headers_object(token),
        }),
        Harness::Pi => serde_json::json!({
            "url": url,
            "headers": headers_object("!qol mcp token"),
        }),
        Harness::Kimi => serde_json::json!({
            "url": url,
            "headers": headers_object(token),
        }),
    }
}

pub(crate) fn apply_json_entry(document: &str, entry: serde_json::Value) -> Result<String> {
    let mut root: serde_json::Value = serde_json::from_str(document)
        .map_err(|error| anyhow!("invalid JSON config document: {error}"))?;
    let servers = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("config document is not a JSON object"))?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    servers
        .as_object_mut()
        .ok_or_else(|| anyhow!("mcpServers is not a JSON object"))?
        .insert("qol".to_owned(), entry);
    let mut updated = serde_json::to_string_pretty(&root)
        .map_err(|error| anyhow!("failed to serialize the config document: {error}"))?;
    updated.push('\n');
    Ok(updated)
}

fn codex_entry_table(url: &str, token: &str) -> toml_edit::Table {
    let mut entry = toml_edit::Table::new();
    entry.insert("url", toml_edit::value(url));
    let mut headers = toml_edit::InlineTable::new();
    headers.insert(HTTP_AUTH_HEADER, token.into());
    entry.insert("http_headers", toml_edit::value(headers));
    entry
}

pub(crate) fn codex_entry_text(url: &str, token: &str) -> Result<String> {
    apply_codex_entry("", url, token)
}

pub(crate) fn apply_codex_entry(document: &str, url: &str, token: &str) -> Result<String> {
    let mut document: toml_edit::DocumentMut = document
        .parse()
        .map_err(|error| anyhow!("invalid codex config TOML: {error}"))?;
    let servers = document
        .as_table_mut()
        .entry("mcp_servers")
        .or_insert_with(|| {
            let mut servers = toml_edit::Table::new();
            servers.set_implicit(true);
            toml_edit::Item::Table(servers)
        })
        .as_table_mut()
        .ok_or_else(|| anyhow!("codex config mcp_servers is not a table"))?;
    servers.remove("qol");
    servers.insert("qol", toml_edit::Item::Table(codex_entry_table(url, token)));
    Ok(document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_conventions::{DEFAULT_PORT, LOCAL_HOST};

    fn test_url() -> String {
        format!("http://{LOCAL_HOST}:{DEFAULT_PORT}/api/mcp")
    }

    #[test]
    fn parse_accepts_the_four_harness_names_and_rejects_others() {
        assert!(matches!(Harness::parse("claude"), Some(Harness::Claude)));
        assert!(matches!(Harness::parse("codex"), Some(Harness::Codex)));
        assert!(matches!(Harness::parse("pi"), Some(Harness::Pi)));
        assert!(matches!(Harness::parse("kimi"), Some(Harness::Kimi)));
        assert!(Harness::parse("").is_none());
        assert!(Harness::parse("Claude").is_none());
        assert!(Harness::parse("claude-code").is_none());
        assert!(Harness::parse("cursor").is_none());
    }

    #[test]
    fn json_entry_matches_the_documented_shapes() {
        let url = test_url();
        let token = "token-value";
        let claude = json_entry(Harness::Claude, &url, token);
        assert_eq!(claude["type"], "http");
        assert_eq!(claude["url"], url.as_str());
        assert_eq!(claude["headersHelper"], "qol mcp headers");
        let pi = json_entry(Harness::Pi, &url, token);
        assert_eq!(pi["url"], url.as_str());
        assert_eq!(pi["headers"][HTTP_AUTH_HEADER], "!qol mcp token");
        let kimi = json_entry(Harness::Kimi, &url, token);
        assert_eq!(kimi["url"], url.as_str());
        assert_eq!(kimi["headers"][HTTP_AUTH_HEADER], token);
        let codex = json_entry(Harness::Codex, &url, token);
        assert_eq!(codex["url"], url.as_str());
        assert_eq!(codex["http_headers"][HTTP_AUTH_HEADER], token);
    }

    #[test]
    fn apply_json_entry_creates_mcp_servers_when_absent() {
        let url = test_url();
        let entry = json_entry(Harness::Kimi, &url, "token-value");
        let output = apply_json_entry("{}", entry.clone()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["mcpServers"]["qol"], entry);
        assert!(output.ends_with('\n'));
    }

    #[test]
    fn apply_json_entry_preserves_unrelated_servers_and_replaces_qol() {
        let url = test_url();
        let document = r#"{"mcpServers":{"other":{"url":"http://elsewhere"},"qol":{"url":"http://stale"}},"unrelated":true}"#;
        let entry = json_entry(Harness::Pi, &url, "token-value");
        let output = apply_json_entry(document, entry.clone()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["mcpServers"]["other"]["url"], "http://elsewhere");
        assert_eq!(parsed["mcpServers"]["qol"], entry);
        assert_eq!(parsed["unrelated"], true);
    }

    #[test]
    fn apply_codex_entry_preserves_other_tables_and_comments() {
        let url = test_url();
        let document = "# top comment\n\n[mcp_servers.other]\nurl = \"http://elsewhere\"\n";
        let output = apply_codex_entry(document, &url, "token-value").unwrap();
        assert!(output.contains("# top comment"));
        assert!(output.contains("[mcp_servers.qol]"));
        let parsed = output.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            parsed["mcp_servers"]["qol"]["url"].as_str(),
            Some(url.as_str())
        );
        assert_eq!(
            parsed["mcp_servers"]["qol"]["http_headers"][HTTP_AUTH_HEADER].as_str(),
            Some("token-value")
        );
        assert_eq!(
            parsed["mcp_servers"]["other"]["url"].as_str(),
            Some("http://elsewhere")
        );
    }
}
