use crate::keycode::macos_keycode::PhysicalLayout;
use crate::layout::LayoutError;

pub struct KeyLayout {
    id: String,
}

impl KeyLayout {
    pub fn current() -> Result<Self, LayoutError> {
        Err(LayoutError::Unsupported)
    }

    pub fn by_id(_id: &str) -> Result<Self, LayoutError> {
        Err(LayoutError::Unsupported)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn translate(&self, _code: u16, _state: u32, _dead_key_state: &mut u32) -> Option<String> {
        None
    }
}

pub fn current_layout_id() -> Result<String, LayoutError> {
    Err(LayoutError::Unsupported)
}

pub fn physical_layout() -> PhysicalLayout {
    PhysicalLayout::Ansi
}
