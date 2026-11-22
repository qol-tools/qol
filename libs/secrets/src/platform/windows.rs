use anyhow::{bail, Result};
use qol_platform::native::wide::wide_nul;
use windows_sys::Win32::Security::Credentials::{
    CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_MAX_CREDENTIAL_BLOB_SIZE,
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};
use zeroize::Zeroizing;

pub(crate) fn read(service: &str) -> Result<String> {
    let target = wide_nul(service);
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `target` is a NUL-terminated UTF-16 string that outlives the call,
    // and `credential` is a valid out pointer.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &raw mut credential) } == 0 {
        bail!(
            "the Credential Manager has no readable generic credential `{service}`: {}",
            std::io::Error::last_os_error()
        );
    }
    // SAFETY: CredReadW succeeded, so `credential` points at a CREDENTIALW whose
    // blob is `CredentialBlobSize` bytes long until CredFree releases it.
    let blob = unsafe {
        let credential = &*credential;
        std::slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        )
        .to_vec()
    };
    // SAFETY: `credential` was allocated by CredReadW and is freed exactly once.
    unsafe { CredFree(credential.cast()) };
    decode(&blob, service)
}

pub(crate) fn store(service: &str, secret: &str) -> Result<()> {
    let mut target = wide_nul(service);
    let mut blob = encode(secret);
    let Ok(size) = u32::try_from(blob.len()) else {
        bail!("the secret `{service}` is too long for the Credential Manager");
    };
    if size > CRED_MAX_CREDENTIAL_BLOB_SIZE {
        bail!(
            "the secret `{service}` is {size} bytes as UTF-16, more than the \
             {CRED_MAX_CREDENTIAL_BLOB_SIZE} the Credential Manager holds"
        );
    }
    let mut credential: CREDENTIALW = unsafe { std::mem::zeroed() };
    credential.Type = CRED_TYPE_GENERIC;
    credential.TargetName = target.as_mut_ptr();
    credential.CredentialBlobSize = size;
    credential.CredentialBlob = blob.as_mut_ptr();
    credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
    if unsafe { CredWriteW(&raw const credential, 0) } == 0 {
        bail!(
            "the Credential Manager did not store the generic credential `{service}`: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

fn encode(secret: &str) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(secret.encode_utf16().flat_map(u16::to_le_bytes).collect())
}

fn decode(blob: &[u8], service: &str) -> Result<String> {
    if !blob.contains(&0) {
        return String::from_utf8(blob.to_vec())
            .map_err(|_| anyhow::anyhow!("the credential `{service}` is not UTF-8"));
    }
    if !blob.len().is_multiple_of(2) {
        bail!("the credential `{service}` is neither UTF-8 nor UTF-16");
    }
    let units: Vec<u16> = blob
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units)
        .map_err(|_| anyhow::anyhow!("the credential `{service}` is not UTF-16"))
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn a_blob_is_read_as_utf8_unless_it_carries_utf16_nul_bytes() {
        assert_eq!(decode(b"sk-ant-oat01", "s").unwrap(), "sk-ant-oat01");
        let utf16: Vec<u8> = "sk-ant".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode(&utf16, "s").unwrap(), "sk-ant");
        assert!(decode(&[b'a', 0, b'b'], "s").is_err());
    }

    #[test]
    fn a_stored_blob_reads_back_as_the_same_secret() {
        for secret in ["sk-ant-oat01-AbC_123", "a", "with inner space"] {
            assert_eq!(decode(&encode(secret), "s").unwrap(), secret);
        }
    }
}
