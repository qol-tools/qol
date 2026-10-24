use super::catalog::{self, InstalledPlugin};
use anyhow::{anyhow, bail, Context, Result};
use qol_host_defaults::{Ledger, Role};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Opened {
    pub plugin_id: String,
    pub args: Vec<String>,
}

pub(super) fn open(ledger: &Ledger, role: Role, args: &[String]) -> Result<Opened> {
    let holder = ledger.holder(role)?.ok_or_else(|| no_holder(role))?;
    let plugin = catalog::load_plugin(&holder)
        .ok_or_else(|| anyhow!("{holder} holds {} but is no longer installed", role.id()))?;
    let declaration = plugin
        .declaration_for(role)
        .ok_or_else(|| anyhow!("{holder} no longer declares {}", role.id()))?;
    let mut invocation = runtime_args(&plugin, &declaration.action)?;
    invocation.extend(target_args(args));
    spawn(&plugin, &invocation)?;
    Ok(Opened {
        plugin_id: holder,
        args: invocation,
    })
}

fn no_holder(role: Role) -> anyhow::Error {
    anyhow!(
        "no plugin holds {}; the system handler is in place again and qol-open will not hand the request back to it",
        role.id()
    )
}

fn runtime_args(plugin: &InstalledPlugin, action: &str) -> Result<Vec<String>> {
    let manifest = &plugin.manifest;
    if let Some(args) = manifest.catalog_runtime_args(action) {
        return Ok(args);
    }
    let runtime = manifest
        .runtime
        .as_ref()
        .ok_or_else(|| anyhow!("{} has no runtime command", plugin.id))?;
    match runtime.actions.as_ref() {
        None => Ok(vec![action.to_string()]),
        Some(actions) => actions
            .get(action)
            .cloned()
            .ok_or_else(|| anyhow!("{} maps no runtime args for {action}", plugin.id)),
    }
}

fn spawn(plugin: &InstalledPlugin, args: &[String]) -> Result<()> {
    let runtime = plugin
        .manifest
        .runtime
        .as_ref()
        .ok_or_else(|| anyhow!("{} has no runtime command", plugin.id))?;
    let binary = crate::plugins::resolve_plugin_command_path_for_source(
        &plugin.root,
        &runtime.command,
        None,
    )
    .ok_or_else(|| anyhow!("{} binary {} was not found", plugin.id, runtime.command))?;
    let mut command = std::process::Command::new(binary);
    command
        .args(args)
        .current_dir(&plugin.root)
        .env(qol_conventions::ENV_PLUGIN_ID, &plugin.id)
        .env(
            qol_conventions::ENV_STATE_SOCKET,
            crate::dev_generation::state_socket_path(),
        );
    qol_process::hide_console_window(&mut command);
    crate::features::theme::apply_accent_env(&mut command);
    crate::features::theme::apply_theme_name_env(&mut command);
    qol_process::spawn_detached(&mut command)
        .with_context(|| format!("starting {}", runtime.command))
}

pub(super) fn target_args(args: &[String]) -> Vec<String> {
    args.iter().map(|arg| file_uri_to_path(arg)).collect()
}

pub(super) fn file_uri_to_path(value: &str) -> String {
    let Some(rest) = value.strip_prefix("file://") else {
        return value.to_string();
    };
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return value.to_string();
    }
    let decoded = percent_decode(path);
    strip_drive_slash(decoded)
}

fn strip_drive_slash(path: String) -> String {
    let bytes = path.as_bytes();
    let drive =
        bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    if drive {
        return path[1..].to_string();
    }
    path
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| hex_pair(bytes.get(index + 1..index + 3)?))
            .flatten();
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_pair(pair: &[u8]) -> Option<u8> {
    let high = (pair[0] as char).to_digit(16)?;
    let low = (pair[1] as char).to_digit(16)?;
    u8::try_from(high * 16 + low).ok()
}

pub(super) fn require_target(args: &[String]) -> Result<()> {
    if args.is_empty() {
        bail!("open needs at least one path or file:// uri");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_pass_through() {
        assert_eq!(file_uri_to_path("/home/a/b"), "/home/a/b");
        assert_eq!(file_uri_to_path("relative/dir"), "relative/dir");
    }

    #[test]
    fn file_uris_become_decoded_paths() {
        let cases = [
            ("file:///home/a/My%20Docs", "/home/a/My Docs"),
            ("file://localhost/tmp/x", "/tmp/x"),
            ("file:///caf%C3%A9", "/café"),
            ("file:///C:/Users/a", "C:/Users/a"),
            ("file:///100%25", "/100%"),
        ];
        for (uri, expected) in cases {
            assert_eq!(file_uri_to_path(uri), expected, "{uri}");
        }
    }

    #[test]
    fn a_bad_escape_stays_literal() {
        assert_eq!(file_uri_to_path("file:///a%zzb%"), "/a%zzb%");
    }

    #[test]
    fn other_schemes_and_hosts_are_left_alone() {
        for value in ["https://example.org/x", "file://remote/x", "file:relative"] {
            assert_eq!(file_uri_to_path(value), value);
        }
    }

    #[test]
    fn a_missing_target_is_refused() {
        assert!(require_target(&[]).is_err());
        assert!(require_target(&["/tmp".to_string()]).is_ok());
    }
}
