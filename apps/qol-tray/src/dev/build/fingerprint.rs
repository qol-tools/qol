use std::path::Path;

mod hash;
mod inputs;
mod path_deps;

pub(crate) fn fingerprint_plugin(path: &Path) -> Result<String, String> {
    let mut inputs = inputs::fingerprint_inputs(path)?;
    if inputs.is_empty() {
        return Err("No Rust build inputs found".to_string());
    }
    for dep_inputs in path_deps::collect_path_dep_inputs(path) {
        inputs.extend(dep_inputs);
    }
    hash::hash_inputs(inputs)
}
