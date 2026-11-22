use std::fmt::Write as _;
use std::process::Command;

use anyhow::{bail, Context, Result};
use zeroize::Zeroizing;

const SECURITY: &str = "/usr/bin/security";
const STORE: &str = "the login Keychain";
const MAX_LINE_LEN: usize = 4095;
const SECRET_FLAG: &str = " -X ";

pub(crate) fn read(service: &str) -> Result<String> {
    let mut command = Command::new(SECURITY);
    command.args(["find-generic-password", "-s", service, "-w"]);
    super::tool::secret_from(command, service, STORE)
}

pub(crate) fn store(service: &str, secret: &str) -> Result<()> {
    let account = std::env::var("USER")
        .context("USER is not set, and the Keychain entry is stored under that account")?;
    let line = store_line(&account, service, secret)?;
    let hex = line
        .rfind(SECRET_FLAG)
        .map_or("", |start| line[start + SECRET_FLAG.len()..].trim_end());
    let mut command = Command::new(SECURITY);
    command.arg("-i");
    super::tool::store_through_stdin(command, line.as_bytes(), &[secret, hex], service, STORE)
}

fn store_line(account: &str, service: &str, secret: &str) -> Result<Zeroizing<String>> {
    if account.is_empty() || !account.bytes().all(|byte| byte.is_ascii_graphic()) {
        bail!(
            "the Keychain account `{}` is not printable ASCII",
            account.escape_default()
        );
    }
    let mut line = Zeroizing::new(String::with_capacity(MAX_LINE_LEN + 1));
    line.push_str("add-generic-password -U -a ");
    push_quoted(&mut line, account);
    line.push_str(" -s ");
    push_quoted(&mut line, service);
    line.push_str(SECRET_FLAG);
    let hex_len = secret.len() * 2;
    if line.len() + hex_len + 1 > MAX_LINE_LEN {
        bail!(
            "the secret `{service}` with its name and account is longer than the \
             {MAX_LINE_LEN} bytes {SECURITY} reads per command"
        );
    }
    for byte in secret.bytes() {
        let _ = write!(line, "{byte:02x}");
    }
    line.push('\n');
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

    #[derive(PartialEq)]
    enum State {
        SkipWhitespace,
        ReadArg,
        ReadArgEscaped,
        QuotedArg,
        QuotedArgEscaped,
    }

    fn security_split_line(line: &str) -> Vec<String> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let mut words = Vec::new();
        let mut word = String::new();
        let mut state = State::SkipWhitespace;
        let mut quote = '"';
        for character in line.chars() {
            if state == State::SkipWhitespace {
                if character.is_ascii_whitespace() {
                    continue;
                }
                if matches!(character, '"' | '\'') {
                    quote = character;
                    state = State::QuotedArg;
                    continue;
                }
                state = State::ReadArg;
            }
            if state == State::ReadArg {
                if character == '\\' {
                    state = State::ReadArgEscaped;
                    continue;
                } else if character.is_ascii_whitespace() {
                    words.push(std::mem::take(&mut word));
                    state = State::SkipWhitespace;
                } else {
                    word.push(character);
                }
            }
            if state == State::QuotedArg {
                if character == '\\' {
                    state = State::QuotedArgEscaped;
                    continue;
                }
                if character == quote {
                    words.push(std::mem::take(&mut word));
                    state = State::SkipWhitespace;
                } else {
                    word.push(character);
                }
            }
            if state == State::ReadArgEscaped {
                word.push(character);
                state = State::ReadArg;
            }
            if state == State::QuotedArgEscaped {
                word.push(character);
                state = State::QuotedArg;
            }
        }
        if state != State::SkipWhitespace {
            words.push(word);
        }
        words
    }

    fn hex(text: &str) -> String {
        text.bytes().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn the_command_matches_add_generic_password_by_hand_with_the_secret_as_hex() {
        let line = store_line("tester", "claude-oauth-work", "sk-ant-oat01-AbC").unwrap();
        assert_eq!(
            line.as_str(),
            format!(
                "add-generic-password -U -a \"tester\" -s \"claude-oauth-work\" -X {}\n",
                hex("sk-ant-oat01-AbC")
            )
        );
    }

    #[test]
    fn security_splits_every_hostile_name_account_and_secret_into_the_same_eight_words() {
        let hostile = [
            "plain",
            "-A",
            "\" -A",
            "x\" -A -T \"",
            "back\\slash\\",
            "\\\" -A",
            "$HOME `id` $(id)",
            "'single' \"double\"",
            "-T /Applications/Evil.app",
            "a\\\\\" -A \"b",
        ];
        for name in hostile {
            for secret in hostile {
                let line = store_line(name, name, secret)
                    .unwrap_or_else(|_| store_line("tester", name, secret).unwrap());
                let words = security_split_line(&line);
                let account = if name.bytes().all(|byte| byte.is_ascii_graphic()) {
                    name
                } else {
                    "tester"
                };
                assert_eq!(
                    words,
                    [
                        "add-generic-password",
                        "-U",
                        "-a",
                        account,
                        "-s",
                        name,
                        "-X",
                        hex(secret).as_str(),
                    ],
                    "{name:?} / {secret:?}"
                );
            }
        }
    }

    #[test]
    fn a_line_security_would_cut_short_is_refused_without_the_secret() {
        let secret = "s".repeat(MAX_LINE_LEN / 2);
        let error = format!("{:#}", store_line("tester", "name", &secret).unwrap_err());
        assert!(!error.contains(&secret[..32]), "{error}");
        assert!(!error.contains(&hex(&secret[..32])), "{error}");
        let fits = "s".repeat(1280);
        assert!(store_line("tester", &"n".repeat(256), &fits).unwrap().len() <= MAX_LINE_LEN);
        assert!(store_line("two words", "name", "secret").is_err());
    }
}
