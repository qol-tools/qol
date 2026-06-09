use std::collections::HashSet;

use super::super::Environment;

pub(crate) fn dedupe_and_sort(mut environments: Vec<Environment>) -> Vec<Environment> {
    environments.sort_by(|a, b| {
        source_rank(&a.source)
            .cmp(&source_rank(&b.source))
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.source.cmp(&b.source))
    });
    let mut seen_ids = HashSet::new();
    let mut seen_paths = HashSet::new();
    let mut unique = Vec::new();
    for environment in &mut environments {
        let path_key = environment
            .image_path
            .canonicalize()
            .unwrap_or_else(|_| environment.image_path.clone());
        if !seen_paths.insert(path_key) {
            continue;
        }
        if seen_ids.insert(environment.id.clone()) {
            unique.push(environment.clone());
            continue;
        }
        let base = environment.id.clone();
        for index in 2.. {
            let candidate = format!("{base}-{index}");
            if seen_ids.insert(candidate.clone()) {
                environment.id = candidate;
                break;
            }
        }
        unique.push(environment.clone());
    }
    unique.sort_by(|a, b| a.id.cmp(&b.id));
    unique
}

fn source_rank(source: &str) -> usize {
    match source {
        "config" => 0,
        source if source.starts_with("libvirt:") => 1,
        "scan" => 2,
        _ => 3,
    }
}
