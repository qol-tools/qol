pub mod app;
pub mod audio;
pub mod cli;
pub mod config;
pub mod listen;
pub mod transcribe;
pub mod turn;
pub mod voice_session;

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
