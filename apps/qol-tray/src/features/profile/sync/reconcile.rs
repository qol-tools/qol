use anyhow::{anyhow, Result};
use std::path::Path;

use super::git_repo::GitRepo;
use super::merge::{merge_profile, ProfileMerge, ProfileSnapshot};
use crate::features::profile::ProfileScopeStore;

pub(crate) fn mergeable_path(rel: &Path) -> bool {
    ProfileScopeStore::is_sync_allowlisted(rel)
        && rel.extension().map(|ext| ext == "json").unwrap_or(false)
        && !rel.components().any(|c| c.as_os_str() == "sync")
}

pub(crate) fn reconcile(repo: &GitRepo) -> Result<ProfileMerge> {
    let local_oid = repo
        .local_oid()?
        .ok_or_else(|| anyhow!("local branch has no commit"))?;
    let remote_oid = repo.remote_oid()?;
    let base = match repo.merge_base_with_remote()? {
        Some(oid) => repo.snapshot_json_at(oid, mergeable_path)?,
        None => Default::default(),
    };
    let local = repo.snapshot_json_at(local_oid, mergeable_path)?;
    let remote = repo.snapshot_json_at(remote_oid, mergeable_path)?;
    Ok(merge_profile(
        &ProfileSnapshot { files: base },
        &ProfileSnapshot { files: local },
        &ProfileSnapshot { files: remote },
    ))
}

#[cfg(test)]
mod tests {
    use super::super::git_repo::{GitRepo, SignatureSpec};
    use super::*;
    use git2::Repository;
    use std::path::Path;
    use tempfile::TempDir;

    fn init_bare_origin(dir: &Path) -> String {
        Repository::init_bare(dir).unwrap();
        let normalized = dir.display().to_string().replace('\\', "/");
        format!("file:///{}", normalized.trim_start_matches('/'))
    }

    fn write_file(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn sig() -> SignatureSpec {
        SignatureSpec {
            name: "Tester".to_string(),
            email: "tester@example.com".to_string(),
        }
    }

    #[test]
    fn mergeable_path_accepts_core_os_manifest_and_excludes_backups_and_gitignore() {
        let cases = [
            ("default/manifest.json", true),
            ("default/core/plugin-configs/plugin-alt-tab.json", true),
            ("default/os/macos/plugin-configs/plugin-lights.json", true),
            ("default/sync/backups/20260508-conflict.json", false),
            (".gitignore", false),
            ("default/device/plugin-configs/x.json", false),
        ];
        for (raw, want) in cases {
            assert_eq!(mergeable_path(Path::new(raw)), want, "path: {raw}");
        }
    }

    #[test]
    fn reconcile_auto_merges_independent_changes_and_flags_only_real_clashes() {
        let tmp = TempDir::new().unwrap();
        let url = init_bare_origin(&tmp.path().join("o.git"));
        let alt_tab = "default/core/plugin-configs/plugin-alt-tab.json";
        let lights = "default/core/plugin-configs/plugin-lights.json";

        let a_path = tmp.path().join("a");
        let a = GitRepo::init(&a_path, &url).unwrap();
        write_file(&a_path.join(alt_tab), "{\"opacity\":1.0}");
        a.commit_all("seed", &sig()).unwrap();
        a.push(None).unwrap();

        let b_path = tmp.path().join("b");
        let b = GitRepo::clone(&url, &b_path, None).unwrap();

        write_file(&a_path.join(alt_tab), "{\"opacity\":0.5}");
        a.commit_all("a2", &sig()).unwrap();
        a.push(None).unwrap();

        write_file(&b_path.join(alt_tab), "{\"opacity\":0.8}");
        write_file(&b_path.join(lights), "{\"theme\":\"warm\"}");
        b.commit_all("b2", &sig()).unwrap();

        b.pull(None).unwrap();
        let merge = reconcile(&b).unwrap();

        assert_eq!(merge.conflicts.len(), 1, "only opacity clashes");
        assert_eq!(merge.conflicts[0].key_path, "opacity");
        assert_eq!(merge.conflicts[0].plugin.as_deref(), Some("plugin-alt-tab"));
        assert!(
            merge.merged.contains_key(lights),
            "independent b-only file is auto-merged in"
        );
    }
}
