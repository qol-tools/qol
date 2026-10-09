use super::super::Check;
use super::server::{checks, Host};

struct Linux;

impl Host for Linux {
    const NAME: &'static str = "Linux";
    const START: &'static str = "Start the sound server";
}

pub(crate) const CHECKS: &[Check] = &checks::<Linux>();
