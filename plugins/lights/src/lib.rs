pub mod backend;
pub mod cli;
pub mod config;
pub mod daemon;
mod doctor;
pub mod domain;
pub mod platform;
pub mod runtime;
pub mod service;
pub mod znp;

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
