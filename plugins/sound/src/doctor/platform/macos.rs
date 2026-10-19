use super::super::Check;
use super::server::{checks, Host};

struct Macos;

impl Host for Macos {
    const NAME: &'static str = qol_platform::Os::Macos.label();
    const START: &'static str = "Restart Core Audio with `sudo killall coreaudiod`";
}

pub(crate) const CHECKS: &[Check] = &checks::<Macos>();
