#![allow(clippy::print_stdout, clippy::print_stderr)]

use anyhow::{Context, Result};

use super::{parse_args, run_post_auth_blocking};

pub(super) fn run_migrations(args: impl IntoIterator<Item = String>) -> Result<()> {
    let args = parse_args(args)?;
    let config_dir = match args.config_dir {
        Some(dir) => dir,
        None => qol_tray::paths::shared_config_dir().context("locating qol-tray config dir")?,
    };
    if !config_dir.exists() {
        eprintln!("config dir does not exist: {}", config_dir.display());
        return Ok(());
    }

    let pre_flight_reports =
        qol_migrations::run_pre_flight(&config_dir, env!("CARGO_PKG_VERSION"))?;
    print_reports("pre-flight", &pre_flight_reports);

    if args.post_auth {
        run_post_auth_blocking(&config_dir)?;
    }

    if pre_flight_reports.is_empty() && !args.post_auth {
        println!(
            "qol-tray-migrate: nothing to migrate in {}",
            config_dir.display()
        );
    }

    Ok(())
}

fn print_reports(phase: &str, reports: &[qol_migrations::MigrationReport]) {
    for report in reports {
        println!(
            "qol-tray-migrate[{phase}]: applied {} (archived {} paths)",
            report.name,
            report.archived.len(),
        );
        for path in &report.archived {
            println!("    - {}", path.display());
        }
    }
}
