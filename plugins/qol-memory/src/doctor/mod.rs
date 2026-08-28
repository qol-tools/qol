use anyhow::Result;
use qol_headless::{DoctorCheck, DoctorCheckResult};

use crate::retrieval::cache::{cache_state, CacheState};
use crate::retrieval::DocRef;
use crate::retrieval_log::RETRIEVAL_LOG_CAP;
use crate::skills::{load_index, probe_fresh, Freshness};
use crate::store::Store;

pub fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "platform_supported",
            "Verify the current platform is declared by the plugin.",
            platform_supported_check,
        ),
        DoctorCheck::new(
            "agent_homes",
            "Verify every registered agent home transcript root exists.",
            agent_homes_check,
        ),
        DoctorCheck::new(
            "store_dir",
            "Verify the memory store directory exists.",
            store_dir_check,
        ),
        DoctorCheck::new(
            "units_layer",
            "Verify the units layer has content.",
            units_layer_check,
        ),
        DoctorCheck::new(
            "notes_layer",
            "Verify a notes run exists.",
            notes_layer_check,
        ),
        DoctorCheck::new(
            "index_cache",
            "Verify the user index cache state.",
            index_cache_check,
        ),
        DoctorCheck::new(
            "skills_index",
            "Verify the skills pool index state.",
            skills_index_check,
        ),
        DoctorCheck::new(
            "retrieval_log",
            "Verify the retrieval log size is within cap.",
            retrieval_log_check,
        ),
        DoctorCheck::new(
            "aliases_valid",
            "Verify the embedded concept aliases validate.",
            aliases_valid_check,
        ),
    ]
}

fn agent_homes_check() -> Result<DoctorCheckResult> {
    let registry = qol_agent_homes::Registry::load();
    let mut warnings: Vec<String> = Vec::new();
    if let Some(error) = registry.load_error() {
        let file = qol_config::config_dir()
            .map(|dir| {
                dir.join(qol_agent_homes::REGISTRY_FILE_NAME)
                    .display()
                    .to_string()
            })
            .unwrap_or_else(|| qol_agent_homes::REGISTRY_FILE_NAME.to_string());
        warnings.push(format!("{file} could not be loaded: {error}"));
    }
    let roots = crate::ingest::IngestRoots::from_registry(&registry);
    let mut listed: Vec<String> = Vec::new();
    let mut absent: Vec<String> = Vec::new();
    let mut broken: Vec<String> = Vec::new();
    for root in &roots.roots {
        let home_path = registry
            .homes()
            .iter()
            .find(|home| home.id == root.agent_home)
            .map(|home| home.path.clone())
            .unwrap_or_else(|| {
                root.path
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_else(|| root.path.clone())
            });
        if !home_path.exists() {
            absent.push(root.agent_home.clone());
        } else if !root.path.is_dir() {
            broken.push(format!("{} -> {}", root.agent_home, root.path.display()));
        } else {
            listed.push(format!("{} -> {}", root.agent_home, root.path.display()));
        }
    }
    if !broken.is_empty() {
        warnings.push(format!(
            "transcript root directories are missing: {}",
            broken.join(", ")
        ));
    }
    let mut unregistered: Vec<String> = Vec::new();
    for harness in [
        qol_agent_homes::Harness::Claude,
        qol_agent_homes::Harness::Pi,
    ] {
        let current = registry.current(harness);
        if !registry.is_registered(&current.id) {
            warnings.push(format!(
                "the current {} home {} is not registered",
                harness.id(),
                current.id
            ));
            unregistered.push(format!(
                "qol agents add {} {}",
                harness.id(),
                current.path.display()
            ));
        }
    }
    if warnings.is_empty() {
        let mut parts = listed;
        if !absent.is_empty() {
            parts.push(format!("absent homes: {}", absent.join(", ")));
        }
        parts.push("a caller sees its own home plus shared homes".to_string());
        Ok(DoctorCheckResult::ok(
            "agent_homes",
            format!("Agent homes and transcript roots: {}.", parts.join("; ")),
        ))
    } else {
        let result = DoctorCheckResult::warn("agent_homes", warnings.join("; "));
        let result = if unregistered.is_empty() {
            result
        } else {
            result.with_fix(format!("register it with {}", unregistered.join(" or ")))
        };
        Ok(result)
    }
}

fn store_dir_check() -> Result<DoctorCheckResult> {
    Ok(match Store::resolve(None) {
        Ok(store) => {
            if store.root().exists() {
                DoctorCheckResult::ok(
                    "store_dir",
                    format!("Memory store found at {}.", store.root().display()),
                )
            } else {
                DoctorCheckResult::warn("store_dir", "no memory yet")
                    .with_fix("run a session with live capture or ingest a snapshot")
            }
        }
        Err(error) => DoctorCheckResult::fail(
            "store_dir",
            format!("Memory store could not be resolved: {error:#}"),
        ),
    })
}

fn units_layer_check() -> Result<DoctorCheckResult> {
    let store = match Store::resolve(None) {
        Ok(store) => store,
        Err(error) => {
            return Ok(DoctorCheckResult::warn(
                "units_layer",
                format!("Skipped: failed to resolve the memory store: {error:#}"),
            ))
        }
    };
    Ok(match store.read_units() {
        Ok(layer) => DoctorCheckResult::ok(
            "units_layer",
            format!("{} units read from {}.", layer.items.len(), layer.run),
        ),
        Err(error) => {
            DoctorCheckResult::warn("units_layer", format!("No units layer yet: {error:#}"))
        }
    })
}

fn notes_layer_check() -> Result<DoctorCheckResult> {
    let store = match Store::resolve(None) {
        Ok(store) => store,
        Err(error) => {
            return Ok(DoctorCheckResult::warn(
                "notes_layer",
                format!("Skipped: failed to resolve the memory store: {error:#}"),
            ))
        }
    };
    Ok(match store.read_notes() {
        Ok(layer) => match layer.run {
            Some(run) => {
                DoctorCheckResult::ok("notes_layer", format!("Notes present from run {run}."))
            }
            None => DoctorCheckResult::warn("notes_layer", "No notes run yet."),
        },
        Err(error) => {
            DoctorCheckResult::warn("notes_layer", format!("No notes layer yet: {error:#}"))
        }
    })
}

fn index_cache_check() -> Result<DoctorCheckResult> {
    let store = match Store::resolve(None) {
        Ok(store) => store,
        Err(error) => {
            return Ok(DoctorCheckResult::warn(
                "index_cache",
                format!("Skipped: failed to resolve the memory store: {error:#}"),
            ))
        }
    };
    let layer = match store.read_units() {
        Ok(layer) => layer,
        Err(error) => {
            return Ok(DoctorCheckResult::warn(
                "index_cache",
                format!("Skipped: no units layer yet: {error:#}"),
            ))
        }
    };
    let registry = qol_agent_homes::Registry::load();
    let caller = registry.resolve_caller(None);
    let slug = crate::agent_home::cache_slug(&caller);
    let user_units_input: Vec<crate::store::Unit> = layer
        .items
        .iter()
        .filter(|unit| unit.kind == "user" && crate::agent_home::visible(unit, &caller, &registry))
        .cloned()
        .collect();
    let user_units = crate::store::dedupe_user_units(&user_units_input);
    let refs: Vec<DocRef> = user_units
        .iter()
        .map(|unit| DocRef {
            key: unit.key.as_str(),
            text: unit.text.as_str(),
        })
        .collect();
    let layer_name = format!("user-{slug}");
    Ok(
        match cache_state(store.root(), &layer_name, &refs, Some(&layer.path)) {
            CacheState::Fresh => DoctorCheckResult::ok(
                "index_cache",
                format!("User index cache for caller {caller} matches the current units layer."),
            ),
            CacheState::Stale => DoctorCheckResult::warn(
                "index_cache",
                format!("User index cache for caller {caller} is stale; rebuilt on next ask."),
            ),
            CacheState::Missing => DoctorCheckResult::warn(
                "index_cache",
                format!("No user index cache for caller {caller}; built on next ask."),
            ),
        },
    )
}

fn skills_index_check() -> Result<DoctorCheckResult> {
    let store = match Store::resolve(None) {
        Ok(store) => store,
        Err(error) => {
            return Ok(DoctorCheckResult::warn(
                "skills_index",
                format!("Skipped: failed to resolve the memory store: {error:#}"),
            ))
        }
    };
    let Some(index) = load_index(&store.skills_index_path()) else {
        return Ok(
            DoctorCheckResult::warn("skills_index", "Skills pool is not indexed.")
                .with_fix("node docs/research/qol-memory/skills.mjs"),
        );
    };
    let skills_root = index
        .root
        .as_ref()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("QOL_MEMORY_SKILLS_ROOT")
                .filter(|raw| !raw.is_empty())
                .map(std::path::PathBuf::from)
        });
    let Some(skills_root) = skills_root else {
        return Ok(DoctorCheckResult::warn(
            "skills_index",
            "Skills pool root is unavailable: no index root and no QOL_MEMORY_SKILLS_ROOT.",
        ));
    };
    Ok(match probe_fresh(&index, &skills_root) {
        Freshness::Fresh => DoctorCheckResult::ok("skills_index", "Skills pool index is fresh."),
        Freshness::Stale => DoctorCheckResult::warn("skills_index", "Skills pool index is stale.")
            .with_fix("node docs/research/qol-memory/skills.mjs"),
        Freshness::Unavailable => {
            DoctorCheckResult::warn("skills_index", "Skills pool root is unavailable.")
        }
        Freshness::NotIndexed => {
            DoctorCheckResult::warn("skills_index", "Skills pool has no walked_at timestamp.")
                .with_fix("node docs/research/qol-memory/skills.mjs")
        }
    })
}

fn retrieval_log_check() -> Result<DoctorCheckResult> {
    let store = match Store::resolve(None) {
        Ok(store) => store,
        Err(error) => {
            return Ok(DoctorCheckResult::warn(
                "retrieval_log",
                format!("Skipped: failed to resolve the memory store: {error:#}"),
            ))
        }
    };
    Ok(match std::fs::metadata(store.retrievals_path()) {
        Ok(meta) => {
            let bytes = meta.len();
            if bytes > RETRIEVAL_LOG_CAP {
                DoctorCheckResult::warn(
                    "retrieval_log",
                    format!("Retrieval log is {bytes} bytes, over cap."),
                )
            } else {
                DoctorCheckResult::ok(
                    "retrieval_log",
                    format!("Retrieval log is within cap at {bytes} bytes."),
                )
            }
        }
        Err(_) => DoctorCheckResult::ok("retrieval_log", "No retrieval log yet."),
    })
}

fn aliases_valid_check() -> Result<DoctorCheckResult> {
    let problems = crate::aliases::validate(crate::aliases::CONCEPT_ALIASES_JSON);
    Ok(if problems.is_empty() {
        DoctorCheckResult::ok("aliases_valid", "Embedded concept aliases validate.")
    } else {
        DoctorCheckResult::fail(
            "aliases_valid",
            format!(
                "{} concept alias problem(s): {}",
                problems.len(),
                problems.join("; ")
            ),
        )
    })
}

fn platform_supported_check() -> Result<DoctorCheckResult> {
    Ok(platform_supported_result(crate::platform::current_support()))
}

fn platform_supported_result(support: crate::platform::PlatformSupport) -> DoctorCheckResult {
    if support.supported {
        return DoctorCheckResult::ok(
            "platform_supported",
            format!("{} is supported.", support.name),
        );
    }
    DoctorCheckResult::fail(
        "platform_supported",
        format!("{} is not declared by this plugin.", support.name),
    )
    .with_fix("Run the plugin on Linux or macOS.")
}
