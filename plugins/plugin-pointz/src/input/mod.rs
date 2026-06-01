//! Cross-platform cursor/keyboard input dispatch.
//!
//! Each `<os>.rs` provides an `InputHandlerImpl` that implements
//! [`InputHandlerTrait`]. The wiring layer below picks the correct impl per
//! target. Unsupported OSes get an `other.rs` stub that returns typed `Err`
//! at construction time, so the binary still compiles and the daemon can
//! report "not supported" cleanly instead of panicking.
//!
//! See `workspace/.claude/skills/qol-architecture` for the pattern rules.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod other;
#[cfg(target_os = "linux")]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

use crate::domain::models::{Command, ModifierKeys};
use anyhow::Result;

#[cfg(target_os = "linux")]
use unix::InputHandlerImpl;

#[cfg(target_os = "macos")]
use macos::InputHandlerImpl;

#[cfg(target_os = "windows")]
use windows::InputHandlerImpl;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use other::InputHandlerImpl;

pub struct InputHandler {
    inner: InputHandlerImpl,
}

impl InputHandler {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: InputHandlerImpl::new()?,
        })
    }

    pub fn handle_command(&self, command: Command) -> Result<()> {
        match command {
            Command::MouseMove { x, y } => self.inner.mouse_move(x, y),
            Command::MouseClick { button } => self.inner.mouse_click(button),
            Command::MouseDown { button } => self.inner.mouse_down(button),
            Command::MouseUp { button } => self.inner.mouse_up(button),
            Command::MouseScroll { delta_x, delta_y } => self.inner.mouse_scroll(delta_x, delta_y),
            Command::KeyPress { key, modifiers } => self.inner.key_press(&key, &modifiers),
            Command::KeyRelease { key, modifiers } => self.inner.key_release(&key, &modifiers),
            Command::ModifierPress { modifier } => self.inner.modifier_press(&modifier),
            Command::ModifierRelease { modifier } => self.inner.modifier_release(&modifier),
        }
    }
}

pub(crate) trait InputHandlerTrait: Send + Sync {
    fn mouse_move(&self, x: f64, y: f64) -> Result<()>;
    fn mouse_click(&self, button: u8) -> Result<()>;
    fn mouse_down(&self, button: u8) -> Result<()>;
    fn mouse_up(&self, button: u8) -> Result<()>;
    fn mouse_scroll(&self, delta_x: f64, delta_y: f64) -> Result<()>;
    fn key_press(&self, key: &str, modifiers: &ModifierKeys) -> Result<()>;
    fn key_release(&self, key: &str, modifiers: &ModifierKeys) -> Result<()>;
    fn modifier_press(&self, modifier: &str) -> Result<()>;
    fn modifier_release(&self, modifier: &str) -> Result<()>;
}
