mod platform;

use anyhow::{bail, Result};
use zeroize::Zeroizing;

pub const MAX_NAME_LEN: usize = 256;

pub const MAX_SECRET_LEN: usize = 1280;

/// Reads the secret stored under `service` in this OS's credential store:
/// the login Keychain on macOS, the Secret Service on Linux, and the
/// Credential Manager on Windows.
pub fn read(service: &str) -> Result<String> {
    if service.trim().is_empty() {
        bail!("a secret needs a non-empty service name");
    }
    let secret = platform::read(service)?;
    let secret = secret.trim_end_matches(['\r', '\n']);
    if secret.is_empty() {
        bail!("the secret `{service}` is empty");
    }
    Ok(secret.to_owned())
}

pub fn store(service: &str, secret: &str) -> Result<()> {
    check_name(service)?;
    check_secret(service, secret)?;
    platform::store(service, secret)?;
    let stored = Zeroizing::new(read(service)?);
    if stored.as_str() != secret {
        bail!(
            "stored the secret `{service}`, but reading `{service}` back finds a different \
             value; another entry with that name shadows it, so remove the other entry and \
             store it again"
        );
    }
    Ok(())
}

fn check_name(service: &str) -> Result<()> {
    if service.is_empty() {
        bail!("a secret needs a non-empty name");
    }
    if service.len() > MAX_NAME_LEN {
        bail!("a secret name is at most {MAX_NAME_LEN} characters");
    }
    if service.starts_with('-') {
        bail!("the secret name `{service}` starts with `-`, which the credential tools read as an option");
    }
    if !service.bytes().all(|byte| byte.is_ascii_graphic()) {
        bail!(
            "a secret name is printable ASCII with no spaces; `{}` is not",
            service.escape_default()
        );
    }
    Ok(())
}

fn check_secret(service: &str, secret: &str) -> Result<()> {
    if secret.is_empty() {
        bail!("the secret for `{service}` is empty");
    }
    if secret.len() > MAX_SECRET_LEN {
        bail!("the secret for `{service}` is longer than {MAX_SECRET_LEN} bytes");
    }
    if !secret
        .bytes()
        .all(|byte| byte == b' ' || byte.is_ascii_graphic())
    {
        bail!(
            "the secret for `{service}` holds a character that is not printable ASCII, such as \
             a line break or a tab; a secret is one line of printable ASCII"
        );
    }
    if secret.starts_with(' ') || secret.ends_with(' ') {
        bail!("the secret for `{service}` starts or ends with a space");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{check_name, check_secret, read, store, MAX_NAME_LEN, MAX_SECRET_LEN};

    const PLANTED: &str = "planted-value-9f3c";

    #[test]
    fn a_blank_service_is_refused_before_the_store_is_asked() {
        let error = read("  ").unwrap_err().to_string();
        assert!(error.contains("non-empty service"), "{error}");
    }

    #[test]
    fn names_the_credential_tools_take_as_one_argument_are_accepted() {
        let longest = "n".repeat(MAX_NAME_LEN);
        for name in [
            "claude-oauth-work",
            "a",
            "x.y_z:1@host/path+=",
            longest.as_str(),
        ] {
            check_name(name).unwrap_or_else(|error| panic!("{name}: {error}"));
        }
    }

    #[test]
    fn names_with_spaces_controls_a_leading_dash_or_no_text_are_refused() {
        let long = "n".repeat(MAX_NAME_LEN + 1);
        for name in [
            "",
            "-w",
            "two words",
            "tab\tname",
            "line\nname",
            "nul\0name",
            "é",
            long.as_str(),
        ] {
            assert!(check_name(name).is_err(), "{name:?} was accepted");
        }
    }

    #[test]
    fn one_line_of_printable_ascii_is_a_secret() {
        let longest = "s".repeat(MAX_SECRET_LEN);
        for secret in [
            "sk-ant-oat01-AbC_123",
            "a",
            "with inner space",
            "!\"#$%&'()*+,-./\\~",
            longest.as_str(),
        ] {
            check_secret("name", secret).unwrap_or_else(|error| panic!("{secret}: {error}"));
        }
    }

    #[test]
    fn a_secret_that_would_not_read_back_unchanged_is_refused() {
        let long = "s".repeat(MAX_SECRET_LEN + 1);
        for secret in [
            "",
            " lead",
            "trail ",
            "line\nbreak",
            "end\n",
            "tab\t",
            "nul\0",
            "é",
            long.as_str(),
        ] {
            assert!(
                check_secret("name", secret).is_err(),
                "{secret:?} was accepted"
            );
        }
    }

    #[test]
    fn a_refusal_never_carries_the_secret() {
        for secret in [
            format!("{PLANTED}\n"),
            format!(" {PLANTED}"),
            format!("{PLANTED}é"),
            format!("{PLANTED}{}", "s".repeat(MAX_SECRET_LEN)),
        ] {
            let error = format!("{:#}", check_secret("name", &secret).unwrap_err());
            assert!(!error.contains(PLANTED), "{error}");
        }
    }

    #[test]
    fn store_refuses_a_bad_name_or_secret_before_the_store_is_asked() {
        let error = format!("{:#}", store("two words", PLANTED).unwrap_err());
        assert!(error.contains("no spaces"), "{error}");
        let error = format!("{:#}", store("name", &format!("{PLANTED}\t")).unwrap_err());
        assert!(
            error.contains("printable ASCII") && !error.contains(PLANTED),
            "{error}"
        );
    }
}
