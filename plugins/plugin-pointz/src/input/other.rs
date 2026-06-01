//! Stub input handler for unsupported OSes.
//!
//! `InputHandlerImpl::new` returns a typed `Err` so the daemon can log and
//! exit gracefully rather than panic. The trait methods are unreachable
//! because the constructor never succeeds, but they still return typed `Err`
//! to satisfy the trait without `unimplemented!()`.

use crate::domain::models::ModifierKeys;
use crate::input::InputHandlerTrait;
use anyhow::{anyhow, Result};

pub struct InputHandlerImpl;

impl InputHandlerImpl {
    pub fn new() -> Result<Self> {
        Err(anyhow!(
            "plugin-pointz: input handling is not implemented on this OS"
        ))
    }
}

fn unsupported<T>() -> Result<T> {
    Err(anyhow!(
        "plugin-pointz: input handling is not implemented on this OS"
    ))
}

impl InputHandlerTrait for InputHandlerImpl {
    fn mouse_move(&self, _x: f64, _y: f64) -> Result<()> {
        unsupported()
    }

    fn mouse_click(&self, _button: u8) -> Result<()> {
        unsupported()
    }

    fn mouse_down(&self, _button: u8) -> Result<()> {
        unsupported()
    }

    fn mouse_up(&self, _button: u8) -> Result<()> {
        unsupported()
    }

    fn mouse_scroll(&self, _delta_x: f64, _delta_y: f64) -> Result<()> {
        unsupported()
    }

    fn key_press(&self, _key: &str, _modifiers: &ModifierKeys) -> Result<()> {
        unsupported()
    }

    fn key_release(&self, _key: &str, _modifiers: &ModifierKeys) -> Result<()> {
        unsupported()
    }

    fn modifier_press(&self, _modifier: &str) -> Result<()> {
        unsupported()
    }

    fn modifier_release(&self, _modifier: &str) -> Result<()> {
        unsupported()
    }
}
