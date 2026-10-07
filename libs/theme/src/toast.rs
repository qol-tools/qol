use crate::{HEIGHT_INLINE, SPACE_CELL, SPACE_PAD, SPACE_STACK, SPACE_TIGHT};

pub const WIDTH: f32 = 440.0;
pub const HEIGHT: f32 = 84.0;
pub const PREVIEW: f32 = 72.0;
pub const CLOSE: f32 = 44.0;
pub const GROW: f32 = 1.2;
pub const FAN: f32 = 2.0 * SPACE_PAD;
pub const SHOW_ALL_ROW: f32 = HEIGHT_INLINE - SPACE_TIGHT;
pub const LEAVE_SCALE: f32 = 0.92;
pub const MESSAGE_ARRIVE_SCALE: f32 = 0.96;
pub const MESSAGE_LEAVE_SCALE: f32 = 0.85;
pub const MESSAGE_HEIGHT: f32 = HEIGHT;
pub const RULE: f32 = SPACE_STACK;
pub const MESSAGE_MARK: f32 = SPACE_PAD;
pub const MESSAGE_MARK_INSET: f32 = SPACE_CELL;
pub const MESSAGE_TEXT_INSET: f32 = 2.0 * SPACE_CELL + SPACE_PAD;
pub const EDGE_FLOOR: f32 = 0.3;
