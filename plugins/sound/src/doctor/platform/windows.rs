use super::super::Check;
use super::server::{checks, Host};

struct Windows;

impl Host for Windows {
    const NAME: &'static str = "Windows";
    const START: &'static str = "Start the Windows Audio service";
}

pub(crate) const CHECKS: &[Check] = &checks::<Windows>();
