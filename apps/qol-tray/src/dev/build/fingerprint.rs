use std::path::Path;

mod hash;
mod inputs;

pub(crate) fn fingerprint_plugin(path: &Path) -> Result<String, String> {
    let inputs = inputs::fingerprint_inputs(path)?;
    if inputs.is_empty() {
        return Err("No Rust build inputs found".to_string());
    }
    hash::hash_inputs(inputs)
}
