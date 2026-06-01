//! Windows stub: session resolution is deferred per the design spec's
//! non-goals.

use std::path::PathBuf;

use crate::resolver::ResolveError;

pub fn resolve_session_jsonl(_pid: u32) -> Result<PathBuf, ResolveError> {
    Err(ResolveError::PlatformUnsupported)
}
