use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use qol_agent_homes::{normalize, AgentHome, Harness, Registry, REGISTRY_FILE_NAME};
use qol_headless::PlainTextOutput;
use serde_json::Value;
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

pub(crate) fn run(args: &[String]) -> Result<PlainTextOutput> {
    match args.first() {
        None => Ok(PlainTextOutput::text(help_text())),
        Some(other) => bail!("unknown qol agents subcommand `{other}`\n\n{}", help_text()),
    }
}

pub(crate) fn list_plain() -> Result<PlainTextOutput> {
    let registry = Registry::load();
    warn_load_error(&registry);
    Ok(PlainTextOutput::text(render_rows(&registry)))
}

pub(crate) fn list_json() -> Result<Value> {
    let registry = Registry::load();
    let homes: Vec<AgentHome> = listed_homes(&registry);
    let mut object = serde_json::Map::new();
    object.insert(
        "homes".to_owned(),
        serde_json::to_value(&homes)
            .map_err(|error| anyhow!("failed to serialize agent homes: {error}"))?,
    );
    if let Some(error) = registry.load_error() {
        object.insert("error".to_owned(), Value::String(error.to_owned()));
    }
    Ok(Value::Object(object))
}

pub(crate) fn current_plain(args: &[String]) -> Result<PlainTextOutput> {
    Ok(PlainTextOutput::text(current_home(args)?.id))
}

pub(crate) fn current_json(args: &[String]) -> Result<Value> {
    serde_json::to_value(current_home(args)?)
        .map_err(|error| anyhow!("failed to serialize the agent home: {error}"))
}

pub(crate) fn add_plain(args: &[String]) -> Result<PlainTextOutput> {
    let mut shared = false;
    let mut default = false;
    let mut positional = Vec::new();
    for arg in filter_delimiter(args) {
        match arg {
            "--shared" => shared = true,
            "--default" => default = true,
            _ => positional.push(arg),
        }
    }
    let [harness_name, raw_path] = positional.as_slice() else {
        bail!("usage: qol agents add <claude|codex|kimi|pi> <path> [--shared] [--default]");
    };
    let harness = parse_harness(harness_name)?;
    let stored_path = raw_path.trim().trim_end_matches('/');
    if stored_path.is_empty() {
        bail!("home path cannot be empty");
    }
    let file = registry_file()?;
    let home = user_home()?;
    add_entry(&file, &home, harness, stored_path, shared, default)?;
    warn_load_error(&Registry::load());
    let id = normalize(stored_path, &home);
    Ok(PlainTextOutput::text(format!(
        "updated {}\n{}",
        file.display(),
        row(harness, &id, shared, default, Origin::Declared)
    )))
}

pub(crate) fn remove_plain(args: &[String]) -> Result<PlainTextOutput> {
    let args = filter_delimiter(args);
    let [raw_path] = args.as_slice() else {
        bail!("usage: qol agents remove <path>");
    };
    if raw_path.trim().is_empty() {
        bail!("home path cannot be empty");
    }
    let file = registry_file()?;
    let home = user_home()?;
    let harnesses = remove_entry(&file, &home, raw_path)?;
    warn_load_error(&Registry::load());
    let id = normalize(raw_path.trim(), &home);
    Ok(PlainTextOutput::text(format!(
        "removed {id} ({})",
        harnesses.join(", ")
    )))
}

fn current_home(args: &[String]) -> Result<AgentHome> {
    let args = filter_delimiter(args);
    let [name] = args.as_slice() else {
        bail!("usage: qol agents current <claude|codex|kimi|pi> [--json]");
    };
    let harness = parse_harness(name)?;
    Ok(Registry::load().current(harness))
}

fn parse_harness(name: &str) -> Result<Harness> {
    Harness::parse(name)
        .ok_or_else(|| anyhow!("unknown harness `{name}`; expected claude, codex, kimi, or pi"))
}

fn filter_delimiter(args: &[String]) -> Vec<&str> {
    args.iter()
        .map(String::as_str)
        .filter(|arg| *arg != "--")
        .collect()
}

fn registry_file() -> Result<PathBuf> {
    qol_config::config_dir()
        .map(|dir| dir.join(REGISTRY_FILE_NAME))
        .ok_or_else(|| anyhow!("qol config directory not found"))
}

fn user_home() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow!("home directory not found"))
}

fn render_rows(registry: &Registry) -> String {
    let mut rows = String::new();
    for home in registry.homes() {
        let origin = if home.declared {
            Origin::Declared
        } else {
            Origin::Implicit
        };
        rows.push_str(&row(
            home.harness,
            &home.id,
            home.shared,
            home.default,
            origin,
        ));
    }
    for harness in Harness::ALL {
        if let Some(id) = registry.env_home(harness) {
            if !registry.is_registered(id) {
                rows.push_str(&row(harness, id, false, false, Origin::Unregistered));
            }
        }
    }
    rows
}

fn row(harness: Harness, id: &str, shared: bool, default: bool, origin: Origin) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\n",
        harness.id(),
        id,
        if shared { "shared" } else { "-" },
        if default { "default" } else { "-" },
        match origin {
            Origin::Declared => "declared",
            Origin::Implicit => "implicit",
            Origin::Unregistered => "unregistered",
        }
    )
}

#[derive(Clone, Copy)]
enum Origin {
    Declared,
    Implicit,
    Unregistered,
}

fn listed_homes(registry: &Registry) -> Vec<AgentHome> {
    let mut homes: Vec<AgentHome> = registry.homes().to_vec();
    for harness in Harness::ALL {
        if let Some(id) = registry.env_home(harness) {
            if !registry.is_registered(id) {
                homes.push(AgentHome {
                    harness,
                    id: id.to_owned(),
                    path: PathBuf::from(id),
                    shared: false,
                    default: false,
                    declared: false,
                });
            }
        }
    }
    homes
}

fn warn_load_error(registry: &Registry) {
    if let Some(error) = registry.load_error() {
        let file = registry_file().unwrap_or_else(|_| PathBuf::from(REGISTRY_FILE_NAME));
        eprintln!("warning: {}: {error}", file.display());
    }
}

fn add_entry(
    file: &Path,
    user_home: &Path,
    harness: Harness,
    stored_path: &str,
    shared: bool,
    default: bool,
) -> Result<()> {
    let id = normalize(stored_path, user_home);
    let mut document = read_document(file)?;
    let tables = home_tables(&mut document, file)?;
    let mut matched = None;
    for (index, table) in tables.iter().enumerate() {
        let Some((entry_harness, entry_id)) = entry_identity(table, user_home) else {
            continue;
        };
        if entry_harness == harness.id() && entry_id == id {
            matched = Some(index);
            break;
        }
    }
    if let Some(index) = matched {
        if let Some(table) = tables.get_mut(index) {
            set_flag(table, "shared", shared);
            set_flag(table, "default", default);
        }
    } else {
        let mut table = Table::new();
        table["harness"] = value(harness.id());
        table["path"] = value(stored_path);
        set_flag(&mut table, "shared", shared);
        set_flag(&mut table, "default", default);
        tables.push(table);
    }
    let written = matched.unwrap_or(tables.len() - 1);
    if default {
        for (index, table) in tables.iter_mut().enumerate() {
            if index != written && table.get("harness").and_then(Item::as_str) == Some(harness.id())
            {
                table.remove("default");
            }
        }
    }
    write_document(file, &document)
}

fn remove_entry(file: &Path, user_home: &Path, raw_path: &str) -> Result<Vec<String>> {
    let id = normalize(raw_path.trim(), user_home);
    let text = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let mut document = text
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", file.display()))?;
    let Some(tables) = document
        .get_mut("home")
        .and_then(Item::as_array_of_tables_mut)
    else {
        bail!("no [[home]] entries in {}", file.display());
    };
    let mut matched = Vec::new();
    for (index, table) in tables.iter().enumerate() {
        let Some((_, entry_id)) = entry_identity(table, user_home) else {
            continue;
        };
        if entry_id == id {
            matched.push(index);
        }
    }
    if matched.is_empty() {
        bail!("no home entry for `{id}` in {}", file.display());
    }
    let mut harnesses = Vec::new();
    for index in matched.iter().rev() {
        let table = tables.remove(*index);
        if let Some(harness) = table.get("harness").and_then(Item::as_str) {
            harnesses.push(harness.to_owned());
        }
    }
    harnesses.reverse();
    write_document(file, &document)?;
    Ok(harnesses)
}

fn read_document(file: &Path) -> Result<DocumentMut> {
    match std::fs::read_to_string(file) {
        Ok(text) => text
            .parse::<DocumentMut>()
            .with_context(|| format!("failed to parse {}", file.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(error) => {
            Err(anyhow::Error::from(error).context(format!("failed to read {}", file.display())))
        }
    }
}

fn home_tables<'a>(document: &'a mut DocumentMut, file: &Path) -> Result<&'a mut ArrayOfTables> {
    document
        .as_table_mut()
        .entry("home")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or_else(|| anyhow!("`home` in {} is not a [[home]] table array", file.display()))
}

fn entry_identity<'a>(table: &'a Table, user_home: &Path) -> Option<(&'a str, String)> {
    let harness = table.get("harness").and_then(Item::as_str)?;
    let path = table.get("path").and_then(Item::as_str)?;
    Some((harness, normalize(path, user_home)))
}

fn set_flag(table: &mut Table, key: &str, set: bool) {
    if set {
        table[key] = value(true);
    } else {
        table.remove(key);
    }
}

fn write_document(file: &Path, document: &DocumentMut) -> Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    qol_fs::atomic_write(file, document.to_string().as_bytes())
        .with_context(|| format!("failed to write {}", file.display()))
}

fn help_text() -> &'static str {
    r#"qol agents

Inspect and manage the agent home registry.

Usage:
  qol agents list [--json]
  qol agents current <claude|codex|kimi|pi> [--json]
  qol agents add <claude|codex|kimi|pi> <path> [--shared] [--default]
  qol agents remove <path>
  qol agents help

Details:
  list prints every registered home plus unregistered env homes, one
  tab-separated row per home: harness, id, shared or -, default or -, then
  declared, implicit, or unregistered.
  current prints the home id a harness resolves to right now; the harness
  home env var wins when set, otherwise the harness default home. Scripts
  call this verb.
  add appends or updates the [[home]] entry in agents.toml, creating the
  file; --default clears default on that harness's other entries; the
  confirmation is followed by the resulting row.
  remove deletes every [[home]] entry whose path matches, regardless of
  harness, and errors when nothing matched.

See docs/agent-homes.md for the file format and the implicit defaults.
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_render_tab_separated_columns_and_origins() {
        let rows = [
            row(
                Harness::Claude,
                "/home/tester/.claude",
                false,
                true,
                Origin::Implicit,
            ),
            row(
                Harness::Pi,
                "/home/tester/.pi/agent",
                true,
                true,
                Origin::Implicit,
            ),
            row(
                Harness::Claude,
                "/home/tester/.claude-work",
                true,
                false,
                Origin::Declared,
            ),
            row(
                Harness::Codex,
                "/home/tester/work-codex",
                false,
                false,
                Origin::Unregistered,
            ),
        ];
        assert_eq!(
            rows.concat(),
            "claude\t/home/tester/.claude\t-\tdefault\timplicit\n\
             pi\t/home/tester/.pi/agent\tshared\tdefault\timplicit\n\
             claude\t/home/tester/.claude-work\tshared\t-\tdeclared\n\
             codex\t/home/tester/work-codex\t-\t-\tunregistered\n"
        );
    }

    #[test]
    fn add_and_remove_round_trip_on_a_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agents.toml");
        let home = Path::new("/home/tester");
        add_entry(&file, home, Harness::Claude, "~/.claude-work", true, true).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("harness = \"claude\""));
        assert!(text.contains("path = \"~/.claude-work\""));
        assert!(text.contains("shared = true"));
        assert!(text.contains("default = true"));
        add_entry(
            &file,
            home,
            Harness::Claude,
            "~/.claude-work/",
            false,
            false,
        )
        .unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.matches("[[home]]").count(), 1);
        assert!(!text.contains("shared"));
        add_entry(&file, home, Harness::Pi, "~/.claude-work", false, false).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.matches("[[home]]").count(), 2);
        assert!(text.contains("harness = \"pi\""));
        let harnesses = remove_entry(&file, home, "~/.claude-work").unwrap();
        assert_eq!(harnesses, vec!["claude", "pi"]);
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.matches("[[home]]").count(), 0);
        assert!(remove_entry(&file, home, "~/.claude-work").is_err());
    }

    #[test]
    fn add_default_clears_the_harness_other_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agents.toml");
        let home = Path::new("/home/tester");
        add_entry(&file, home, Harness::Kimi, "~/.kimi-one", false, true).unwrap();
        add_entry(&file, home, Harness::Kimi, "~/.kimi-two", false, true).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.matches("[[home]]").count(), 2);
        assert_eq!(text.matches("default = true").count(), 1);
    }

    #[test]
    fn editing_preserves_comments_and_unknown_entries() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agents.toml");
        std::fs::write(
            &file,
            "# my homes\n[[home]]\nharness = \"pi\"\npath = \"~/.pi/agent\"\nshared = true\ndefault = true\n\n[other]\nkey = 1\n",
        )
        .unwrap();
        add_entry(
            &file,
            Path::new("/home/tester"),
            Harness::Claude,
            "~/.claude-work",
            false,
            false,
        )
        .unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("# my homes"));
        assert!(text.contains("[other]"));
        assert_eq!(text.matches("[[home]]").count(), 2);
    }
}
