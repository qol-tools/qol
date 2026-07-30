use crate::artifact::{
    self, BuildIntent, CompilerFacts, SourceIdentity, ENV_BUILD_INTENT,
    ENV_COMPILER_OVERFLOW_CHECKS, ENV_SOURCE_COMMIT, ENV_SOURCE_HEAD_TREE, ENV_SOURCE_WORKING_TREE,
    SCHEMA_VERSION,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Serialize)]
struct BuildIdentityFields {
    schema: u16,
    package: String,
    version: String,
    target: String,
    intent: BuildIntent,
    compiler: CompilerFacts,
    features: Vec<String>,
    source: SourceIdentity,
}

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

    let fields = BuildIdentityFields {
        schema: SCHEMA_VERSION,
        package: required_env("CARGO_PKG_NAME"),
        version: required_env("CARGO_PKG_VERSION"),
        target: required_env("TARGET"),
        intent: build_intent(),
        compiler: compiler_facts(),
        features: enabled_features(),
        source: source_identity(),
    };
    let json = serde_json::to_string(&fields).expect("build identity fields serialize");
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
        Err(_) => return BuildIntent::Unspecified,
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
        Err(_) => None,
    }
}

fn source_identity() -> SourceIdentity {
    let commit = std::env::var(ENV_SOURCE_COMMIT).ok();
    let head_tree = std::env::var(ENV_SOURCE_HEAD_TREE).ok();
    let working_tree = std::env::var(ENV_SOURCE_WORKING_TREE).ok();
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
