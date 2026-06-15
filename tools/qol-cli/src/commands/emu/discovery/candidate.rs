use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::super::{arch::Firmware, arch::GuestArch, humanize_id, Environment};
use super::filesystem::{image_id, is_vm_image_path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageCandidate {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
    pub(crate) display_name: String,
    pub(crate) arch: GuestArch,
    pub(crate) arch_inferred: bool,
    pub(crate) firmware: Firmware,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Discovered {
    pub(crate) environments: Vec<Environment>,
    pub(crate) candidates: Vec<ImageCandidate>,
}

impl Discovered {
    pub(crate) fn partition(environments: Vec<Environment>, emu_dir_entries: &[PathBuf]) -> Self {
        let registered: HashSet<PathBuf> = environments
            .iter()
            .map(|environment| canonical(&environment.image_path))
            .collect();
        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        for entry in emu_dir_entries {
            if !is_vm_image_path(entry) {
                continue;
            }
            let path = canonical(entry);
            if registered.contains(&path) {
                continue;
            }
            if !seen.insert(path.clone()) {
                continue;
            }
            candidates.push(candidate_for(path));
        }
        Self {
            environments,
            candidates,
        }
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn candidate_for(path: PathBuf) -> ImageCandidate {
    let id = image_id(&path);
    let display_name = humanize_id(&id);
    let arch = GuestArch::X86_64;
    ImageCandidate {
        id,
        path,
        display_name,
        arch,
        arch_inferred: false,
        firmware: Firmware::for_arch(arch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_env(id: &str, path: &std::path::Path) -> Environment {
        Environment {
            id: id.to_string(),
            name: humanize_id(id),
            backend: "qemu".to_string(),
            arch: GuestArch::X86_64,
            image_path: path.to_path_buf(),
            source: "config".to_string(),
            firmware: Firmware::for_arch(GuestArch::X86_64),
        }
    }

    #[test]
    fn partition_excludes_registered_paths_and_keeps_unregistered_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registered_file = root.join("registered.qcow2");
        let candidate_file = root.join("fresh.qcow2");
        let not_an_image = root.join("notes.txt");
        for file in [&registered_file, &candidate_file, &not_an_image] {
            fs::write(file, b"x").unwrap();
        }

        let environments = vec![make_env("registered", &registered_file)];
        let entries = vec![
            registered_file.clone(),
            candidate_file.clone(),
            not_an_image.clone(),
        ];

        let discovered = Discovered::partition(environments, &entries);

        assert_eq!(discovered.environments.len(), 1, "registered env preserved");
        assert_eq!(discovered.environments[0].id, "registered");
        assert_eq!(
            discovered.candidates.len(),
            1,
            "only the unregistered image is a candidate"
        );
        let candidate = &discovered.candidates[0];
        assert_eq!(candidate.path, candidate_file.canonicalize().unwrap());
        assert_eq!(candidate.id, "fresh");
        assert_eq!(candidate.display_name, "Fresh");
        assert_eq!(candidate.arch, GuestArch::X86_64);
        assert!(!candidate.arch_inferred);
        assert_eq!(candidate.firmware, Firmware::Bios);
    }

    #[test]
    fn partition_dedups_repeated_entries_and_registered_into_emu_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let registered_image = root.join("vm.qcow2");
        let plain_image = root.join("plain.qcow2");
        for file in [&registered_image, &plain_image] {
            fs::write(file, b"x").unwrap();
        }

        let environments = vec![make_env("vm", &registered_image)];
        let entries = vec![
            registered_image.clone(),
            plain_image.clone(),
            plain_image.clone(),
        ];

        let discovered = Discovered::partition(environments, &entries);

        assert_eq!(
            discovered.candidates.len(),
            1,
            "registered-into-emu_dir not double-listed, repeated entry collapsed"
        );
        assert_eq!(discovered.candidates[0].id, "plain");
    }
}
