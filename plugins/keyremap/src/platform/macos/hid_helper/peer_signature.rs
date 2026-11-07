use std::ffi::c_void;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::ptr;

use anyhow::{ensure, Context, Result};
use core_foundation::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex};
use core_foundation::base::{CFGetTypeID, CFRelease, CFType, CFTypeRef, TCFType};
use core_foundation::data::{CFData, CFDataRef};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};

use super::install::REQUIREMENT_PATH;

const CDHASH_LENGTHS: [usize; 2] = [40, 64];
const SIGNING_INFORMATION: u32 = 1 << 1;
const SOL_LOCAL: i32 = 0;
const LOCAL_PEERTOKEN: i32 = 0x006;
const AUDIT_TOKEN_SIZE: usize = 32;
const SHA1_SIZE: usize = 20;

#[link(name = "Security", kind = "framework")]
extern "C" {
    static kSecCodeInfoCertificates: CFStringRef;
    static kSecGuestAttributeAudit: CFStringRef;
    fn SecCodeCopySelf(flags: u32, code: *mut CFTypeRef) -> i32;
    fn SecCodeCopySigningInformation(
        code: CFTypeRef,
        flags: u32,
        information: *mut CFDictionaryRef,
    ) -> i32;
    fn SecCertificateCopyData(certificate: CFTypeRef) -> CFDataRef;
    fn SecRequirementCreateWithString(
        text: CFStringRef,
        flags: u32,
        requirement: *mut CFTypeRef,
    ) -> i32;
    fn SecCodeCopyGuestWithAttributes(
        host: CFTypeRef,
        attributes: CFDictionaryRef,
        flags: u32,
        guest: *mut CFTypeRef,
    ) -> i32;
    fn SecCodeCheckValidity(code: CFTypeRef, flags: u32, requirement: CFTypeRef) -> i32;
}

extern "C" {
    fn CC_SHA1(data: *const c_void, len: u32, digest: *mut u8) -> *mut u8;
    fn getsockopt(socket: i32, level: i32, name: i32, value: *mut c_void, len: *mut u32) -> i32;
}

pub(super) struct Signer(CFTypeRef);

impl Signer {
    pub(super) fn of_this_helper() -> Result<Self> {
        let certificate = own_leaf_certificate()?;
        let leaf = leaf_requirement(certificate.bytes())?;
        let client = pinned_client_requirement(Path::new(REQUIREMENT_PATH))?;
        Self::from_requirement(&trusted_requirement(&leaf, &client))
    }

    fn from_requirement(text: &str) -> Result<Self> {
        let text = CFString::new(text);
        let mut requirement = ptr::null();
        let status = unsafe {
            SecRequirementCreateWithString(text.as_concrete_TypeRef(), 0, &raw mut requirement)
        };
        ensure!(
            status == 0 && !requirement.is_null(),
            "the code requirement `{text}` is invalid ({status})"
        );
        Ok(Self(requirement))
    }

    pub(super) fn verify(&self, stream: &UnixStream) -> Result<()> {
        let token = peer_audit_token(stream)?;
        let key = unsafe { CFString::wrap_under_get_rule(kSecGuestAttributeAudit) };
        let attributes =
            CFDictionary::from_CFType_pairs(&[(key, CFData::from_buffer(&token).as_CFType())]);
        let mut code = ptr::null();
        let status = unsafe {
            SecCodeCopyGuestWithAttributes(
                ptr::null(),
                attributes.as_concrete_TypeRef(),
                0,
                &raw mut code,
            )
        };
        ensure!(
            status == 0 && !code.is_null(),
            "cannot identify the connecting process ({status})"
        );
        let code = unsafe { CFType::wrap_under_create_rule(code) };
        let status = unsafe { SecCodeCheckValidity(code.as_CFTypeRef(), 0, self.0) };
        ensure!(
            status == 0,
            "the connecting process is not signed with the helper's certificate ({status})"
        );
        Ok(())
    }
}

impl Drop for Signer {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

fn own_leaf_certificate() -> Result<CFData> {
    let mut code = ptr::null();
    let status = unsafe { SecCodeCopySelf(0, &raw mut code) };
    ensure!(
        status == 0 && !code.is_null(),
        "cannot read the helper's own code ({status})"
    );
    let code = unsafe { CFType::wrap_under_create_rule(code) };
    let mut information = ptr::null();
    let status = unsafe {
        SecCodeCopySigningInformation(
            code.as_CFTypeRef(),
            SIGNING_INFORMATION,
            &raw mut information,
        )
    };
    ensure!(
        status == 0 && !information.is_null(),
        "cannot read the helper's signature ({status})"
    );
    let information: CFDictionary<CFString, CFType> =
        unsafe { CFDictionary::wrap_under_create_rule(information) };
    let key = unsafe { CFString::wrap_under_get_rule(kSecCodeInfoCertificates) };
    let certificates = information
        .find(&key)
        .map(|certificates| certificates.as_CFTypeRef())
        .filter(|&certificates| unsafe { CFGetTypeID(certificates) == CFArrayGetTypeID() })
        .filter(|&certificates| unsafe { CFArrayGetCount(certificates.cast()) } > 0)
        .context("the helper is ad-hoc signed, so it has no certificate to trust keyremap by")?;
    let data = unsafe { SecCertificateCopyData(CFArrayGetValueAtIndex(certificates.cast(), 0)) };
    ensure!(!data.is_null(), "cannot read the helper's certificate");
    Ok(unsafe { CFData::wrap_under_create_rule(data) })
}

fn leaf_requirement(certificate: &[u8]) -> Result<String> {
    let len = u32::try_from(certificate.len()).context("the certificate is too large")?;
    let mut digest = [0u8; SHA1_SIZE];
    unsafe { CC_SHA1(certificate.as_ptr().cast(), len, digest.as_mut_ptr()) };
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("certificate leaf = H\"{hex}\""))
}

pub(super) fn is_cdhash(text: &str) -> bool {
    CDHASH_LENGTHS.contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
}

pub(super) fn client_requirement(identifier: &str, cdhash: &str) -> Result<String> {
    ensure!(
        is_identifier(identifier),
        "the client identifier `{identifier}` has characters outside A-Z, a-z, 0-9, `.` and `-`"
    );
    ensure!(
        is_cdhash(cdhash),
        "the client cdhash `{cdhash}` is not 40 or 64 hex characters"
    );
    Ok(format!(
        "identifier \"{identifier}\" and cdhash H\"{cdhash}\""
    ))
}

fn split_client_requirement(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("identifier \"")?;
    let (identifier, rest) = rest.split_once('"')?;
    let cdhash = rest.strip_prefix(" and cdhash H\"")?.strip_suffix('"')?;
    Some((identifier, cdhash))
}

fn validated_client_requirement(text: &str) -> Result<String> {
    let (identifier, cdhash) = split_client_requirement(text.trim())
        .context("it does not read: identifier \"<name>\" and cdhash H\"<hex>\"")?;
    client_requirement(identifier, cdhash)
}

fn pinned_client_requirement(path: &Path) -> Result<String> {
    let text = fs::read_to_string(path).with_context(|| {
        format!(
            "cannot read {}, so no Key Remap build is trusted; run: sudo qol-keyremap install-hid-helper",
            path.display()
        )
    })?;
    validated_client_requirement(&text).with_context(|| {
        format!(
            "{} is malformed; run: sudo qol-keyremap install-hid-helper",
            path.display()
        )
    })
}

fn trusted_requirement(leaf: &str, client: &str) -> String {
    format!("{leaf} and ({client})")
}

fn peer_audit_token(stream: &UnixStream) -> Result<[u8; AUDIT_TOKEN_SIZE]> {
    let mut token = [0u8; AUDIT_TOKEN_SIZE];
    let mut len = AUDIT_TOKEN_SIZE as u32;
    let result = unsafe {
        getsockopt(
            stream.as_raw_fd(),
            SOL_LOCAL,
            LOCAL_PEERTOKEN,
            token.as_mut_ptr().cast(),
            &raw mut len,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error()).context("cannot read the peer's audit token");
    }
    ensure!(
        len as usize == AUDIT_TOKEN_SIZE,
        "the peer's audit token has {len} bytes"
    );
    Ok(token)
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::install::CLIENT_IDENTIFIER;
    use super::*;

    pub(in crate::platform::macos::hid_helper) fn trusting_anyone() -> Signer {
        Signer::from_requirement("always").unwrap()
    }

    pub(in crate::platform::macos::hid_helper) fn trusting_no_one() -> Signer {
        Signer::from_requirement("never").unwrap()
    }

    #[test]
    fn the_requirement_names_the_sha1_of_the_leaf_certificate() {
        assert_eq!(
            leaf_requirement(b"abc").unwrap(),
            "certificate leaf = H\"a9993e364706816aba3e25717850c26c9cd0d89d\""
        );
    }

    const SHA1_CDHASH: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn the_client_requirement_names_the_identifier_and_the_cdhash() {
        assert_eq!(
            client_requirement(CLIENT_IDENTIFIER, SHA1_CDHASH).unwrap(),
            format!("identifier \"com.qol-tools.keyremap\" and cdhash H\"{SHA1_CDHASH}\"")
        );
        assert!(client_requirement(CLIENT_IDENTIFIER, &"ab".repeat(32)).is_ok());
    }

    #[test]
    fn the_trusted_requirement_needs_the_leaf_and_the_pinned_client() {
        let leaf = leaf_requirement(b"abc").unwrap();
        let client = client_requirement(CLIENT_IDENTIFIER, SHA1_CDHASH).unwrap();
        let composed = trusted_requirement(&leaf, &client);
        assert_eq!(composed, format!("{leaf} and ({client})"));
        assert!(Signer::from_requirement(&composed).is_ok());
    }

    #[test]
    fn a_good_requirement_file_is_accepted_with_surrounding_whitespace() {
        let text = format!("identifier \"com.qol-tools.keyremap\" and cdhash H\"{SHA1_CDHASH}\"\n");
        assert_eq!(
            validated_client_requirement(&text).unwrap(),
            text.trim().to_string()
        );
    }

    #[test]
    fn a_requirement_file_without_a_cdhash_or_with_bad_characters_is_refused() {
        let cdhash_missing = "identifier \"com.qol-tools.keyremap\"";
        let wrong_length = "identifier \"com.qol-tools.keyremap\" and cdhash H\"abcdef\"";
        let not_hex = format!(
            "identifier \"com.qol-tools.keyremap\" and cdhash H\"{}\"",
            "zz".repeat(20)
        );
        let quote_in_identifier =
            format!("identifier \"a\" or anchor apple and cdhash H\"{SHA1_CDHASH}\"");
        let space_in_identifier = format!("identifier \"com qol\" and cdhash H\"{SHA1_CDHASH}\"");
        let extra_clause = format!(
            "identifier \"com.qol-tools.keyremap\" and cdhash H\"{SHA1_CDHASH}\" or anchor apple"
        );
        let empty_identifier = format!("identifier \"\" and cdhash H\"{SHA1_CDHASH}\"");
        for text in [
            cdhash_missing,
            wrong_length,
            not_hex.as_str(),
            quote_in_identifier.as_str(),
            space_in_identifier.as_str(),
            extra_clause.as_str(),
            empty_identifier.as_str(),
            "",
        ] {
            assert!(validated_client_requirement(text).is_err(), "{text}");
        }
    }

    #[test]
    fn a_missing_or_malformed_requirement_file_names_the_install_command() {
        let directory =
            std::env::temp_dir().join(format!("qol-keyremap-requirement-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let missing = directory.join("missing");
        let error = format!("{:#}", pinned_client_requirement(&missing).unwrap_err());
        assert!(error.contains("install-hid-helper"), "{error}");
        let malformed = directory.join("malformed");
        fs::write(&malformed, "anchor apple").unwrap();
        let error = format!("{:#}", pinned_client_requirement(&malformed).unwrap_err());
        assert!(error.contains("install-hid-helper"), "{error}");
        let good = directory.join("good");
        fs::write(
            &good,
            client_requirement(CLIENT_IDENTIFIER, SHA1_CDHASH).unwrap(),
        )
        .unwrap();
        assert!(pinned_client_requirement(&good).is_ok());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_peer_is_checked_against_the_requirement() {
        let (client, _server) = UnixStream::pair().unwrap();
        assert!(trusting_anyone().verify(&client).is_ok());
        assert!(trusting_no_one().verify(&client).is_err());
    }
}
