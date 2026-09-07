use qol_conventions::artifact::{
    self, BuildFlavor, BuildIdentity, BuildIntent, BuildRole, CompilerFacts, SourceIdentity,
    ENV_BUILD_INTENT, ENV_COMPILER_OVERFLOW_CHECKS, ENV_SOURCE_COMMIT, ENV_SOURCE_HEAD_TREE,
    ENV_SOURCE_WORKING_TREE, SCHEMA_VERSION,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub fn emit_build_identity() {
    for variable in [
        ENV_BUILD_INTENT,
        ENV_COMPILER_OVERFLOW_CHECKS,
        ENV_SOURCE_COMMIT,
        ENV_SOURCE_HEAD_TREE,
        ENV_SOURCE_WORKING_TREE,
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }

    let intent = build_intent();
    let compiler = compiler_facts();
    let features = enabled_features();
    let flavor = build_flavor(intent, &compiler, &features);
    // Serialize the canonical type so additions to the identity schema cannot
    // leave a private build-script mirror behind. The binary macro supplies
    // only the two executable-specific fields after this build script runs.
    let identity = BuildIdentity {
        schema: SCHEMA_VERSION,
        binary: String::new(),
        role: BuildRole::Host,
        package: required_env("CARGO_PKG_NAME"),
        version: required_env("CARGO_PKG_VERSION"),
        target: required_env("TARGET"),
        intent,
        flavor,
        compiler,
        features,
        source: source_identity(),
    };
    let mut fields =
        serde_json::to_value(identity).expect("canonical build identity fields serialize");
    let fields = fields
        .as_object_mut()
        .expect("canonical build identity serializes as an object");
    assert!(fields.remove("binary").is_some());
    assert!(fields.remove("role").is_some());
    let json = serde_json::to_string(fields).expect("canonical build identity fields serialize");
    let fields = json
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .expect("build identity fields serialize as an object");
    let target_os = required_env("CARGO_CFG_TARGET_OS");
    let frame_prefix = format!("{}{{\"binary\":\"", artifact::FRAME_MAGIC);

    println!("cargo:rustc-env={}={fields}", artifact::ENV_FRAME_FIELDS);
    println!(
        "cargo:rustc-env={}={frame_prefix}",
        artifact::ENV_FRAME_PREFIX
    );
    println!(
        "cargo:rustc-env={}={}",
        artifact::ENV_LINK_SECTION,
        artifact::link_section_for_target(&target_os)
    );
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is set in build scripts"))
}

fn build_intent() -> BuildIntent {
    let value = match std::env::var(ENV_BUILD_INTENT) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return BuildIntent::Unspecified,
        Err(error) => panic!("{ENV_BUILD_INTENT} is invalid: {error}"),
    };
    serde_json::from_value(serde_json::Value::String(value.clone()))
        .unwrap_or_else(|_| panic!("{ENV_BUILD_INTENT} has unsupported value {value:?}"))
}

fn compiler_facts() -> CompilerFacts {
    CompilerFacts {
        cargo_profile: required_env("PROFILE"),
        opt_level: required_env("OPT_LEVEL"),
        debuginfo: required_bool_env("DEBUG"),
        debug_assertions: std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some(),
        overflow_checks: optional_bool_env(ENV_COMPILER_OVERFLOW_CHECKS),
        test: false,
    }
}

fn build_flavor(intent: BuildIntent, compiler: &CompilerFacts, features: &[String]) -> BuildFlavor {
    BuildFlavor::derive(intent, compiler, features)
        .unwrap_or_else(|error| panic!("cannot derive artifact flavor: {error}"))
}

fn required_bool_env(name: &str) -> bool {
    match std::env::var(name).as_deref() {
        Ok("true") => true,
        Ok("false") => false,
        Ok(value) => panic!("{name} must be true or false, got {value:?}"),
        Err(_) => panic!("{name} is set in build scripts"),
    }
}

fn optional_bool_env(name: &str) -> Option<bool> {
    match std::env::var(name).as_deref() {
        Ok("true") => Some(true),
        Ok("false") => Some(false),
        Ok(value) => panic!("{name} must be true or false, got {value:?}"),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => panic!("{name} is invalid: {error}"),
    }
}

fn source_identity() -> SourceIdentity {
    let commit = optional_env(ENV_SOURCE_COMMIT);
    let head_tree = optional_env(ENV_SOURCE_HEAD_TREE);
    let working_tree = optional_env(ENV_SOURCE_WORKING_TREE);
    match (commit, head_tree, working_tree) {
        (None, None, None) => SourceIdentity::Unspecified,
        (Some(commit), Some(head_tree), Some(working_tree)) => SourceIdentity::Git {
            commit,
            head_tree,
            working_tree,
        },
        _ => panic!(
            "{ENV_SOURCE_COMMIT}, {ENV_SOURCE_HEAD_TREE}, and {ENV_SOURCE_WORKING_TREE} must be set together"
        ),
    }
}

fn optional_env(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => panic!("{name} is invalid: {error}"),
    }
}

fn enabled_features() -> Vec<String> {
    let manifest = PathBuf::from(required_env("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let contents = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", manifest.display()));
    let document = toml::from_str::<toml::Value>(&contents)
        .unwrap_or_else(|error| panic!("cannot parse {}: {error}", manifest.display()));
    let declared = match document.get("features").and_then(toml::Value::as_table) {
        Some(declared) => declared,
        None => return Vec::new(),
    };

    let mut by_environment = BTreeMap::<String, Vec<String>>::new();
    for feature in declared.keys() {
        let environment = cargo_feature_environment(feature);
        println!("cargo:rerun-if-env-changed={environment}");
        by_environment
            .entry(environment)
            .or_default()
            .push(feature.clone());
    }

    let mut enabled = Vec::new();
    for (environment, features) in by_environment {
        if std::env::var_os(&environment).is_none() {
            continue;
        }
        if features.len() != 1 {
            panic!("active Cargo features collide at {environment}: {features:?}");
        }
        enabled.extend(features);
    }
    enabled.sort();
    enabled
}

fn cargo_feature_environment(feature: &str) -> String {
    let feature = feature
        .chars()
        .map(|character| {
            if character == '-' {
                return '_';
            }
            character.to_ascii_uppercase()
        })
        .collect::<String>();
    format!("CARGO_FEATURE_{feature}")
}
