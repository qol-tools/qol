#[cfg(feature = "dev")]
mod boot_heal {
    use qol_tray::dev::boot_contract::{heal_drift_on_startup, FsBinaryProbe, GitWorktreeLister};
    use qol_tray::installer::boot_environment::default_boot_environment;
    use tempfile::TempDir;

    #[test]
    fn startup_heal_clears_ghost_marker() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(config_dir.join("dev")).unwrap();
        std::fs::write(
            config_dir.join("dev/active-worktree.txt"),
            "definitely-not-a-real-branch",
        )
        .unwrap();

        let env = default_boot_environment();
        let lister = GitWorktreeLister;
        let probe = FsBinaryProbe;
        let report = heal_drift_on_startup(env.as_ref(), &config_dir, &lister, &probe);

        let cleared = report.actions.iter().any(|a| {
            matches!(
                a,
                qol_tray::dev::boot_contract::HealAction::ClearedSelection { .. }
            )
        });
        let failed = !report.failures.is_empty();

        assert!(
            cleared || failed,
            "heal must either clear the ghost marker or surface a failure; report = {:?}",
            (&report.events, &report.actions, &report.failures)
        );

        if cleared {
            let after = std::fs::read_to_string(config_dir.join("dev/active-worktree.txt"))
                .ok()
                .filter(|s| !s.trim().is_empty());
            assert!(after.is_none(), "marker should be cleared after heal");
        }
    }
}
