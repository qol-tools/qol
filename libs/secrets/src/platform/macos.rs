use std::process::Command;

use anyhow::{bail, Context, Result};
use zeroize::Zeroizing;

const SECURITY: &str = "/usr/bin/security";
const STORE: &str = "the login Keychain";
const MAX_LINE_LEN: usize = 4095;

pub(crate) fn read(service: &str) -> Result<String> {
    let mut command = Command::new(SECURITY);
    command.args(["find-generic-password", "-s", service, "-w"]);
    super::tool::secret_from(command, service, STORE)
}

pub(crate) fn store(service: &str, secret: &str) -> Result<()> {
    let account = std::env::var("USER")
        .context("USER is not set, and the Keychain entry is stored under that account")?;
    let line = store_line(&account, service, secret)?;
    let mut command = Command::new(SECURITY);
    command.arg("-i");
    super::tool::store_through_stdin(command, line.as_bytes(), &[secret], service, STORE)
}

fn store_line(account: &str, service: &str, secret: &str) -> Result<Zeroizing<String>> {
    if account.is_empty() || !account.bytes().all(|byte| byte.is_ascii_graphic()) {
        bail!(
            "the Keychain account `{}` is not printable ASCII",
            account.escape_default()
        );
    }
    let mut line = Zeroizing::new(String::from("add-generic-password -U -a "));
    push_quoted(&mut line, account);
    line.push_str(" -s ");
    push_quoted(&mut line, service);
    line.push_str(" -w ");
    push_quoted(&mut line, secret);
    line.push('\n');
    if line.len() > MAX_LINE_LEN {
        bail!(
            "the secret `{service}` with its name and account is longer than the \
             {MAX_LINE_LEN} bytes {SECURITY} reads per command"
        );
    }
    Ok(line)
}

fn push_quoted(line: &mut String, value: &str) {
    line.push('"');
    for character in value.chars() {
        if matches!(character, '"' | '\\') {
            line.push('\\');
        }
        line.push(character);
    }
    line.push('"');
}

#[cfg(test)]
mod tests {
    use super::{store_line, MAX_LINE_LEN};

    #[test]
    fn the_command_matches_add_generic_password_by_hand_with_the_secret_quoted() {
        let line = store_line("tester", "claude-oauth-work", "sk-ant-oat01-AbC").unwrap();
        assert_eq!(
            line.as_str(),
            "add-generic-password -U -a \"tester\" -s \"claude-oauth-work\" -w \"sk-ant-oat01-AbC\"\n"
        );
    }

    #[test]
    fn quotes_and_backslashes_are_escaped_so_security_reads_one_argument() {
        let line = store_line("tester", "a\"b", "x \"y\" \\z' w").unwrap();
        assert_eq!(
            line.as_str(),
            "add-generic-password -U -a \"tester\" -s \"a\\\"b\" -w \"x \\\"y\\\" \\\\z' w\"\n"
        );
    }

    #[test]
    fn a_line_security_would_split_is_refused_without_the_secret() {
        let secret = "s".repeat(MAX_LINE_LEN);
        let error = format!("{:#}", store_line("tester", "name", &secret).unwrap_err());
        assert!(!error.contains(&secret[..32]), "{error}");
        assert!(store_line("two words", "name", "secret").is_err());
    }
}
