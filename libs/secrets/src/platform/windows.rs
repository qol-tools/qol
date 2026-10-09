use anyhow::{bail, Result};
use windows_sys::Win32::Security::Credentials::{
    CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
};

pub(crate) fn read(service: &str) -> Result<String> {
    let target: Vec<u16> = service.encode_utf16().chain(Some(0)).collect();
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
    use super::decode;

    #[test]
    fn a_blob_is_read_as_utf8_unless_it_carries_utf16_nul_bytes() {
        assert_eq!(decode(b"sk-ant-oat01", "s").unwrap(), "sk-ant-oat01");
        let utf16: Vec<u8> = "sk-ant".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode(&utf16, "s").unwrap(), "sk-ant");
        assert!(decode(&[b'a', 0, b'b'], "s").is_err());
    }
}
