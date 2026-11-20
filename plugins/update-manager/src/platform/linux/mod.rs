use std::path::Path;

pub(crate) mod command;
pub(crate) mod packages;
pub(crate) mod privileged;
pub(crate) mod systemd;

const MINT_AUTOMATION: &str = "/usr/bin/mintupdate-automation";
const SHELL: &str = "/bin/sh";
const LABEL: &str = "qol-update-manager";

pub(crate) fn is_mint() -> bool {
    Path::new(MINT_AUTOMATION).exists()
}
