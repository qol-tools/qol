use std::path::Path;

use crate::features::github_auth::oauth_access_token;

/// Run the post-auth migration registry against `config_dir` if a GitHub
/// token is stored. Returns Ok(()) when there is no token (nothing to do).
pub async fn run_post_auth_if_authed(config_dir: &Path) -> anyhow::Result<()> {
    let Some(token) = oauth_access_token() else {
        log::info!("qol-migrations[post-auth]: skipped (no github token stored)");
        return Ok(());
    };
    let http = reqwest::Client::new();
    let ctx = qol_migrations::MigrationContext {
        config_dir,
        github_token: Some(&token),
        http: Some(&http),
        host_version: env!("CARGO_PKG_VERSION"),
    };
    let reports = qol_migrations::run_post_auth(&ctx).await?;
    if reports.is_empty() {
        log::info!("qol-migrations[post-auth]: nothing to apply");
    }
    for report in reports {
        log::info!(
            "qol-migrations[post-auth]: applied {} (archived {} paths)",
            report.name,
            report.archived.len(),
        );
    }
    Ok(())
}
