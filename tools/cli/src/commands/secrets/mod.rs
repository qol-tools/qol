mod input;

use anyhow::{bail, Result};
use qol_headless::{Command, PlainTextOutput};

const USAGE: &str = "qol secrets store <name>";

pub(crate) fn command() -> Command {
    Command::new("secrets")
        .about("Store named secrets in this computer's credential store.")
        .usage(USAGE)
        .detail("A secret is stored once per computer in its own credential store and read from there by name, so the name can be shared while the value never leaves the computer; see docs/secrets.md.")
        .output("A confirmation line on stdout; the prompt and diagnostics on stderr.")
        .exit_behavior("Exits non-zero when the name or secret is refused, the input is cancelled, or the credential store does not store it.")
        .run_plain_text(|context| run(context.args()))
        .subcommand(
            Command::new("store")
                .about("Store a secret under a name, replacing the one stored under that name.")
                .usage(USAGE)
                .detail("With a terminal on stdin, store prompts for the secret on the terminal without echoing it: Enter stores, Backspace deletes, Ctrl+U clears, and Esc or Ctrl+C cancels without storing. Without a terminal it reads the secret from stdin to its end, such as from a password manager's command piped in. The secret is never an argument, so it never shows in the process list.")
                .detail("Spaces and line breaks around the input are dropped. The secret must then be one line of printable ASCII, at most 1280 characters, and the name printable ASCII with no spaces, at most 256 characters, not starting with `-`.")
                .detail("Linux stores it in the Secret Service through secret-tool (from libsecret-tools) as an item with the label <name> and the attribute service=<name>. macOS stores it in the login Keychain through /usr/bin/security as a generic password with service <name> and account $USER. Windows stores it in the Credential Manager as a generic credential with target <name>, kept for this computer. Each is the entry qol reads the secret from, and an entry stored with the same name by hand (`secret-tool store --label=<name> service <name>`, or `security add-generic-password -U -a \"$USER\" -s <name> -w`) is the same entry, which store replaces.")
                .detail("After storing, store reads the secret back by name and fails when what it reads differs, which happens when another entry with that name shadows the new one.")
                .output("`stored <name>` on stdout; the prompt and diagnostics on stderr. The secret is never printed.")
                .exit_behavior("Exits non-zero when the name or secret is refused, the input is cancelled or empty, stdin is not UTF-8, the credential tool is missing or fails, or the secret read back differs.")
                .run_plain_text(|context| store(context.args())),
        )
}

fn run(args: &[String]) -> Result<PlainTextOutput> {
    match args.first() {
        None => bail!("usage: {USAGE}; run `qol secrets help` for details"),
        Some(other) => bail!("unknown qol secrets subcommand `{other}`; usage: {USAGE}"),
    }
}

fn store(args: &[String]) -> Result<PlainTextOutput> {
    let name = store_name(args)?;
    let secret = input::read_secret(name)?;
    qol_secrets::store(name, &secret)?;
    Ok(PlainTextOutput::text(format!("stored {name}")))
}

fn store_name(args: &[String]) -> Result<&str> {
    let positional: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|arg| *arg != "--")
        .collect();
    match positional.as_slice() {
        [name] => Ok(*name),
        [] => bail!("usage: {USAGE}"),
        _ => bail!(
            "usage: {USAGE}; store takes only the name, and reads the secret from a prompt or stdin"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::store_name;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn store_takes_exactly_one_name() {
        assert_eq!(store_name(&args(&["claude-work"])).unwrap(), "claude-work");
        assert_eq!(
            store_name(&args(&["--", "claude-work"])).unwrap(),
            "claude-work"
        );
        assert!(store_name(&args(&[])).is_err());
    }

    #[test]
    fn a_second_argument_is_refused_without_echoing_it() {
        let error = store_name(&args(&["claude-work", "planted-value-9f3c"]))
            .unwrap_err()
            .to_string();
        assert!(!error.contains("planted-value-9f3c"), "{error}");
        assert!(
            error.contains("reads the secret from a prompt or stdin"),
            "{error}"
        );
    }
}
