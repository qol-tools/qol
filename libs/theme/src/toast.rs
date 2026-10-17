use crate::{HEIGHT_CONTROL, HEIGHT_INLINE, SPACE_CELL, SPACE_PAD, SPACE_STACK, SPACE_TIGHT};

pub const WIDTH: f32 = 440.0;
pub const HEIGHT: f32 = 84.0;
pub const PREVIEW: f32 = 72.0;
pub const CLOSE: f32 = 44.0;
pub const GROW: f32 = 1.2;
pub const FAN: f32 = 2.0 * SPACE_PAD;
pub const SHOW_ALL_ROW: f32 = HEIGHT_INLINE - SPACE_TIGHT;
pub const LEAVE_SCALE: f32 = 0.92;
pub const MESSAGE_HEIGHT: f32 = HEIGHT;
pub const MESSAGE_RISE: f32 = SPACE_CELL;
pub const MESSAGE_RULE_TRACE: f32 = 0.25;
pub const RULE: f32 = SPACE_STACK;
pub const EDGE_FLOOR: f32 = 0.3;
pub const MARK: f32 = SPACE_PAD;
pub const CLOSE_HEIGHT: f32 = 2.0 * SPACE_CELL + MARK;
pub const ACTION_BAR: f32 = HEIGHT_CONTROL;
pub const HEIGHT_STEP: f32 = SPACE_TIGHT;
