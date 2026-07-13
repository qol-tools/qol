mod executor;
mod mimeapps;
mod model;
mod output;
mod planner;
mod platform;

use anyhow::{bail, Result};
use platform::PlatformOps;

pub(super) fn run(
    dry_run: bool,
    json: bool,
    purge_data: bool,
    skip_shell_hook: bool,
) -> Result<()> {
    let options = model::Options {
        dry_run,
        json,
        purge_data,
        skip_shell_hook,
    };
    let platform = platform::Platform;
    let context = platform.context()?;
    let plugins = platform.managed_processes();
    let plan = planner::build(context.clone(), plugins, options);
    let report = if options.dry_run {
        planner::planned_report(&plan)
    } else {
        executor::execute(&platform, &context, &plan)
    };
    output::print(&report, options.json)?;
    if report.is_partial() {
        bail!("uninstall was incomplete; review skipped and failed actions above")
    }
    Ok(())
}
