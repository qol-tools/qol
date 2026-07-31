use anyhow::Result;

mod platform;

pub fn available() -> bool {
    platform::available()
}

pub fn run_privileged(label: &str, script: &str, args: &[String]) -> Result<()> {
    platform::run(label, script, args)
}
