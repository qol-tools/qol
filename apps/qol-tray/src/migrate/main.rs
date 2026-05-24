use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("qol-tray-migrate: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args = parse_args(std::env::args().skip(1))?;
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

fn run_post_auth_blocking(config_dir: &std::path::Path) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime for post-auth")?;
    rt.block_on(qol_tray::migrations_startup::run_post_auth_if_authed(
        config_dir,
    ))
}

struct Args {
    config_dir: Option<PathBuf>,
    post_auth: bool,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args> {
    let mut config_dir = None;
    let mut post_auth = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config-dir" => {
                let value = args
                    .next()
                    .context("--config-dir requires a path argument")?;
                config_dir = Some(PathBuf::from(value));
            }
            "--post-auth" => post_auth = true,
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok(Args {
        config_dir,
        post_auth,
    })
}

fn print_usage() {
    println!("qol-tray-migrate");
    println!();
    println!("USAGE:");
    println!("    qol-tray-migrate                       Run pre-flight migrations on the default config dir");
    println!("    qol-tray-migrate --config-dir <PATH>   Use a specific config dir");
    println!("    qol-tray-migrate --post-auth           Also run cloud (post-auth) migrations.");
    println!(
        "                                           Requires a GitHub token stored by qol-tray;"
    );
    println!("                                           silently no-ops when not signed in.");
    println!("    qol-tray-migrate --help, -h            Print this message and exit");
}
