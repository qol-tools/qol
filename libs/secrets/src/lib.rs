mod platform;

use anyhow::{bail, Result};

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

#[cfg(test)]
mod tests {
    use super::read;

    #[test]
    fn a_blank_service_is_refused_before_the_store_is_asked() {
        let error = read("  ").unwrap_err().to_string();
        assert!(error.contains("non-empty service"), "{error}");
    }
}
