use std::path::Path;

pub(crate) mod command;
pub(crate) mod packages;
pub(crate) mod privileged;

const MINT_AUTOMATION: &str = "/usr/bin/mintupdate-automation";

pub(crate) fn is_mint() -> bool {
    Path::new(MINT_AUTOMATION).exists()
}
