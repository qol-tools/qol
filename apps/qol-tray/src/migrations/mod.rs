use std::path::Path;

use crate::features::github_auth::oauth_access_token;

pub async fn run_post_auth_if_authed(config_dir: &Path) -> anyhow::Result<bool> {
    let Some(token) = oauth_access_token() else {
        log::info!("qol-migrations[post-auth]: skipped (no github token stored)");
        return Ok(false);
    };
    let http = reqwest::Client::new();
    let ctx = qol_migrations::MigrationContext {
        config_dir,
        github_token: Some(&token),
        http: Some(&http),
        host_version: env!("CARGO_PKG_VERSION"),
    };
    let reports = qol_migrations::run_post_auth(&ctx).await?;
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
