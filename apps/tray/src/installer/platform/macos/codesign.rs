use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

const AD_HOC: &str = "-";
const LOCAL_IDENTITY_NAME: &str = "QoL Tray Local Signing";
const LOGIN_KEYCHAIN: &str = "Library/Keychains/login.keychain-db";
const SYSTEM_OPENSSL: &str = "/usr/bin/openssl";
const CERTIFICATE_DAYS: &str = "7300";
const TRANSIENT_PASSWORD: &str = "qol-tray";

pub(crate) fn codesign_bundle(bundle_root: &Path) {
    let identity = signing_identity();
    let output = std::process::Command::new("codesign")
        .args(codesign_args(&identity, bundle_root))
        .output();
    match output {
        Ok(out) if out.status.success() => {
            log::info!("codesigned {} as {identity}", bundle_root.display());
        }
        Ok(out) => log::warn!(
            "codesign as {identity} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => log::warn!("codesign not available: {e}"),
    }
}

fn signing_identity() -> String {
    configured_identity()
        .or_else(local_identity)
        .unwrap_or_else(|| AD_HOC.to_string())
}

fn configured_identity() -> Option<String> {
    let identity = qol_config::codesign_identity()?;
    remember_identity(&identity);
    Some(identity)
}

fn local_identity() -> Option<String> {
    match find_or_create_local_identity() {
        Ok(identity) => {
            remember_identity(&identity);
            Some(identity)
        }
        Err(error) => {
            log::warn!("no local signing identity, signing ad-hoc: {error:#}");
            None
        }
    }
}

fn remember_identity(identity: &str) {
    if let Some(path) = identity_path() {
        write_identity(&path, identity);
    }
}

fn find_or_create_local_identity() -> Result<String> {
    let keychain = login_keychain()?;
    if let Some(identity) = find_local_identity(&keychain)? {
        return Ok(identity);
    }
    create_local_identity(&keychain)?;
    log::info!("created the {LOCAL_IDENTITY_NAME} identity in the login keychain");
    find_local_identity(&keychain)?
        .context("the new signing identity is missing from the login keychain")
}

fn login_keychain() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home directory, so the login keychain is unknown")?;
    let keychain = home.join(LOGIN_KEYCHAIN);
    anyhow::ensure!(
        keychain.exists(),
        "no login keychain at {}",
        keychain.display()
    );
    Ok(keychain)
}

fn find_local_identity(keychain: &Path) -> Result<Option<String>> {
    let output = Command::new("security")
        .args(["find-identity", "-p", "codesigning"])
        .arg(keychain)
        .output()
        .context("security find-identity did not run")?;
    Ok(identity_hash(
        &String::from_utf8_lossy(&output.stdout),
        LOCAL_IDENTITY_NAME,
    ))
}

fn identity_hash(listing: &str, name: &str) -> Option<String> {
    let quoted_name = format!("\"{name}\"");
    listing.lines().find_map(|line| {
        let (_, entry) = line.trim().split_once(") ")?;
        let (hash, label) = entry.split_once(' ')?;
        label.starts_with(&quoted_name).then(|| hash.to_string())
    })
}

fn create_local_identity(keychain: &Path) -> Result<()> {
    let work_dir = tempfile::Builder::new()
        .prefix("qol-tray-signing-")
        .tempdir()?;
    let config = work_dir.path().join("identity.cnf");
    let key = work_dir.path().join("identity.key");
    let certificate = work_dir.path().join("identity.crt");
    let bundle = work_dir.path().join("identity.p12");
    std::fs::write(&config, certificate_config(LOCAL_IDENTITY_NAME))?;

    run(Command::new(SYSTEM_OPENSSL)
        .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes"])
        .args(["-days", CERTIFICATE_DAYS, "-config"])
        .arg(&config)
        .arg("-keyout")
        .arg(&key)
        .arg("-out")
        .arg(&certificate))?;
    run(Command::new(SYSTEM_OPENSSL)
        .args(["pkcs12", "-export", "-passout"])
        .arg(format!("pass:{TRANSIENT_PASSWORD}"))
        .arg("-inkey")
        .arg(&key)
        .arg("-in")
        .arg(&certificate)
        .arg("-out")
        .arg(&bundle))?;
    run(Command::new("security")
        .arg("import")
        .arg(&bundle)
        .arg("-k")
        .arg(keychain)
        .args(["-P", TRANSIENT_PASSWORD, "-T", "/usr/bin/codesign"]))
}

fn certificate_config(name: &str) -> String {
    format!(
        "[req]\n\
         distinguished_name = dn\n\
         prompt = no\n\
         x509_extensions = ext\n\
         [dn]\n\
         CN = {name}\n\
         [ext]\n\
         basicConstraints = critical,CA:false\n\
         keyUsage = critical,digitalSignature\n\
         extendedKeyUsage = critical,codeSigning\n"
    )
}

fn run(command: &mut Command) -> Result<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command
        .output()
        .with_context(|| format!("{program} did not run"))?;
    if !output.status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn codesign_args(identity: &str, bundle_root: &Path) -> Vec<OsString> {
    vec![
        OsString::from("--force"),
        OsString::from("--sign"),
        OsString::from(identity),
        bundle_root.as_os_str().to_os_string(),
    ]
}

fn identity_path() -> Option<PathBuf> {
    qol_config::codesign_identity_path()
}

fn write_identity(path: &Path, identity: &str) {
    if qol_config::codesign_identity_at(path).as_deref() == Some(identity) {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, identity);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qol-codesign-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn configured_identity_is_signed_with_instead_of_ad_hoc() {
        let args = codesign_args("Test Signing Identity", Path::new("/tmp/QoL Tray.app"));
        assert_eq!(args[1], OsString::from("--sign"));
        assert_eq!(args[2], OsString::from("Test Signing Identity"));
        assert_ne!(args[2], OsString::from(AD_HOC));
    }

    #[test]
    fn bundle_path_is_the_last_argument() {
        let bundle = Path::new("/Users/someone/Applications/QoL Tray.app");
        let args = codesign_args("Test Signing Identity", bundle);
        assert_eq!(args.last().unwrap(), bundle.as_os_str());
    }

    #[test]
    fn local_identity_hash_is_read_from_the_find_identity_listing() {
        let listing = "\nPolicy: Code Signing\n  Matching identities\n  \
            1) AAAA1111 \"QoL Tray Local Signing 2\" (CSSMERR_TP_NOT_TRUSTED)\n  \
            2) 216709FA627D87B4 \"QoL Tray Local Signing\" (CSSMERR_TP_NOT_TRUSTED)\n     \
            2 identities found\n\n  Valid identities only\n     0 valid identities found\n";
        assert_eq!(
            identity_hash(listing, LOCAL_IDENTITY_NAME).as_deref(),
            Some("216709FA627D87B4")
        );
        assert_eq!(
            identity_hash("     0 identities found\n", LOCAL_IDENTITY_NAME),
            None
        );
    }

    #[test]
    fn local_certificate_is_a_leaf_limited_to_code_signing() {
        let config = certificate_config(LOCAL_IDENTITY_NAME);
        assert!(config.contains("CN = QoL Tray Local Signing\n"));
        assert!(config.contains("extendedKeyUsage = critical,codeSigning\n"));
        assert!(config.contains("basicConstraints = critical,CA:false\n"));
    }

    #[test]
    fn identity_round_trips_through_the_config_file() {
        let dir = scratch_dir("round-trip");
        let path = dir.join(qol_conventions::CODESIGN_IDENTITY_FILE);

        assert_eq!(qol_config::codesign_identity_at(&path), None);
        write_identity(&path, "Test Signing Identity");
        assert_eq!(
            qol_config::codesign_identity_at(&path).as_deref(),
            Some("Test Signing Identity")
        );

        write_identity(&path, "OTHER");
        assert_eq!(
            qol_config::codesign_identity_at(&path).as_deref(),
            Some("OTHER")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remembering_creates_the_config_directory() {
        let dir = scratch_dir("create-parent");
        let path = dir
            .join("nested")
            .join(qol_conventions::CODESIGN_IDENTITY_FILE);

        write_identity(&path, "Test Signing Identity");

        assert_eq!(
            qol_config::codesign_identity_at(&path).as_deref(),
            Some("Test Signing Identity")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
