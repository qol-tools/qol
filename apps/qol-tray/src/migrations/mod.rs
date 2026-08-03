use std::path::Path;

use crate::features::github_auth::oauth_access_token;

struct ProfileConfigMutationBoundary;

impl qol_migrations::ProfileMutationBoundary for ProfileConfigMutationBoundary {
    fn run(&self, mutation: &mut dyn FnMut() -> anyhow::Result<()>) -> anyhow::Result<()> {
        let _guard = crate::plugins::config::profile_config_write_guard();
        mutation()
    }
}

pub async fn run_post_auth_if_authed(config_dir: &Path) -> anyhow::Result<bool> {
    let Some(token) = oauth_access_token() else {
        log::info!("qol-migrations[post-auth]: skipped (no github token stored)");
        return Ok(false);
    };
    let ctx = qol_migrations::MigrationContext {
        config_dir,
        github_token: Some(&token),
        http: None,
        host_version: env!("CARGO_PKG_VERSION"),
    };
    let reports =
        qol_migrations::run_post_auth_guarded(&ctx, &ProfileConfigMutationBoundary).await?;
    let applied = !reports.is_empty();
    if !applied {
        log::info!("qol-migrations[post-auth]: nothing to apply");
    }
    for report in &reports {
        log::info!(
            "qol-migrations[post-auth]: applied {} (archived {} paths)",
            report.name,
            report.archived.len(),
        );
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_migrations::ProfileMutationBoundary;

    #[test]
    fn failed_profile_mutation_still_invalidates_the_profile_generation() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let before = crate::plugins::config::current_profile_config_generation();
        let mut failing_mutation = || anyhow::bail!("write failed after partial mutation");

        let result = ProfileConfigMutationBoundary.run(&mut failing_mutation);

        assert!(result.is_err());
        assert!(crate::plugins::config::current_profile_config_generation() > before);
    }
}
