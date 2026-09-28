use crate::operations::{
    Failure, OperationBody, MAX_ARGUMENT_BYTES, MAX_BODY_BYTES, MAX_TIMEOUT_MS,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};

pub fn decode_body(document: &str) -> Result<OperationBody, Failure> {
    if document.len() > MAX_BODY_BYTES {
        return Err(Failure::InvalidBody);
    }
    crate::service::framing::validate_operation(document.as_bytes())
        .map_err(|_| Failure::InvalidBody)?;
    let body: OperationBody = serde_json::from_str(document).map_err(|_| Failure::InvalidBody)?;
    if body.version != 1
        || body.timeout_ms == 0
        || body.timeout_ms > MAX_TIMEOUT_MS
        || body.arguments.len() > MAX_ARGUMENT_BYTES
        || body.key.kind == qol_conventions::operations::OperationKind::Stream
    {
        return Err(Failure::InvalidBody);
    }
    super::super::state::validate_grant(&body.key).map_err(|_| Failure::InvalidBody)?;
    crate::service::framing::validate_operation(body.arguments.as_bytes())
        .map_err(|_| Failure::InvalidBody)?;
    let value: serde_json::Value =
        serde_json::from_str(&body.arguments).map_err(|_| Failure::InvalidBody)?;
    if !value.is_null() && !value.is_object() {
        return Err(Failure::InvalidBody);
    }
    Ok(body)
}

pub fn body_digest(document: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"qol-peer-operation-body-v1\0");
    digest.update(document.as_bytes());
    URL_SAFE_NO_PAD.encode(digest.finalize())
}
