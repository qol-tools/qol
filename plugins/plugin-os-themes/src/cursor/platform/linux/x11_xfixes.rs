use anyhow::{bail, Result};

pub struct CursorSession;

impl CursorSession {
    pub fn open(_scale_factor: u32) -> Result<Self> {
        bail!("XFixes shape-preserving cursor not yet implemented")
    }

    pub fn set_scale(&mut self, _scale: f32) -> bool {
        false
    }

    pub fn refresh(&mut self) -> bool {
        false
    }

    pub fn restore(&mut self) {}
}
