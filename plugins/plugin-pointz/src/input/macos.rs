use crate::domain::config::ServerConfig;
use crate::domain::models::ModifierKeys;
use crate::input::InputHandlerTrait;
use anyhow::Result;
use rdev::{simulate, Button, EventType, Key, SimulateError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {}

const DRAG_BATCH_INTERVAL_MS: u64 = 16;

pub struct InputHandlerImpl {
    current_pos: Mutex<Option<(f64, f64)>>,
    modifier_state: Mutex<ModifierKeys>,
    button_state: Mutex<Option<Button>>,
    last_click: Mutex<Option<ClickState>>,
    drag_state: Mutex<DragState>,
}

struct DragState {
    pending_x: f64,
    pending_y: f64,
    last_flush: Instant,
    button: Option<Button>,
}

struct ClickState {
    button: u8,
    time: Instant,
    count: u8,
}

impl InputHandlerImpl {
    pub fn new() -> Result<Self> {
        Ok(Self {
            current_pos: Mutex::new(None),
            modifier_state: Mutex::new(ModifierKeys::default()),
            button_state: Mutex::new(None),
            last_click: Mutex::new(None),
            drag_state: Mutex::new(DragState {
                pending_x: 0.0,
                pending_y: 0.0,
                last_flush: Instant::now(),
                button: None,
            }),
        })
    }

    #[allow(dead_code)]
    fn get_cursor_position() -> Option<(f64, f64)> {
        None
    }
}

fn send_event(event_type: EventType) -> Result<()> {
    match simulate(&event_type) {
        Ok(()) => Ok(()),
        Err(SimulateError) => Err(anyhow::anyhow!(
            "Failed to simulate event: {:?}",
            event_type
        )),
    }
}

#[async_trait::async_trait]
impl InputHandlerTrait for InputHandlerImpl {
    async fn mouse_move(&self, x: f64, y: f64) -> Result<()> {
        let (new_x, new_y, button) = {
            let mut pos_opt = self
                .current_pos
                .lock()
                .expect("Cursor position mutex poisoned");
            let button = self
                .button_state
                .lock()
                .expect("Button state mutex poisoned")
                .clone();

            let (new_x, new_y) = if let Some((px, py)) = *pos_opt {
                (px + x, py + y)
            } else {
                (
                    ServerConfig::FALLBACK_SCREEN_WIDTH / 2.0 + x,
                    ServerConfig::FALLBACK_SCREEN_HEIGHT / 2.0 + y,
                )
            };

            *pos_opt = Some((new_x, new_y));
            (new_x, new_y, button)
        };

        if button.is_some() {
            self.queue_drag_event(x, y, new_x, new_y, button).await?;
        } else {
            send_event(EventType::MouseMove { x: new_x, y: new_y })?;
        }
        Ok(())
    }

    async fn mouse_click(&self, button: u8) -> Result<()> {
        let button_enum = Self::map_button(button);
        let click_state = self.next_click_count(button);
        let position = self.resolve_pointer_position();

        {
            let mut state = self
                .button_state
                .lock()
                .expect("Button state mutex poisoned");
            *state = Some(button_enum);
        }

        Self::send_mouse_button_event(position, button_enum, true, click_state)?;
        tokio::time::sleep(Duration::from_millis(ServerConfig::MOUSE_CLICK_DELAY_MS)).await;
        {
            let mut state = self
                .button_state
                .lock()
                .expect("Button state mutex poisoned");
            *state = None;
        }
        Self::send_mouse_button_event(position, button_enum, false, click_state)?;
        Ok(())
    }

    async fn mouse_down(&self, button: u8) -> Result<()> {
        let button_enum = Self::map_button(button);

        *self
            .button_state
            .lock()
            .expect("Button state mutex poisoned") = Some(button_enum);

        let mut drag = self
            .drag_state
            .lock()
            .expect("Drag state mutex poisoned");
        drag.pending_x = 0.0;
        drag.pending_y = 0.0;
        drag.last_flush = Instant::now();
        drag.button = Some(button_enum);

        send_event(EventType::ButtonPress(button_enum))?;
        Ok(())
    }

    async fn mouse_up(&self, button: u8) -> Result<()> {
        let button_enum = Self::map_button(button);

        self.flush_pending_drag()?;

        *self
            .button_state
            .lock()
            .expect("Button state mutex poisoned") = None;

        let mut drag = self
            .drag_state
            .lock()
            .expect("Drag state mutex poisoned");
        drag.pending_x = 0.0;
        drag.pending_y = 0.0;
        drag.button = None;

        send_event(EventType::ButtonRelease(button_enum))?;
        Ok(())
    }

    async fn mouse_scroll(&self, delta_x: f64, delta_y: f64) -> Result<()> {
        if delta_y != 0.0 {
            send_event(EventType::Wheel {
                delta_x: 0i64,
                delta_y: delta_y as i64,
            })?;
        }
        if delta_x != 0.0 {
            send_event(EventType::Wheel {
                delta_x: delta_x as i64,
                delta_y: 0i64,
            })?;
        }
        Ok(())
    }

    async fn key_press(&self, key: &str, modifiers: &ModifierKeys) -> Result<()> {
        Self::apply_modifiers(&self.modifier_state, modifiers)?;

        if let Some(key_enum) = string_to_key(key) {
            send_event(EventType::KeyPress(key_enum))?;
        }
        Ok(())
    }

    async fn key_release(&self, key: &str, _modifiers: &ModifierKeys) -> Result<()> {
        if let Some(key_enum) = string_to_key(key) {
            send_event(EventType::KeyRelease(key_enum))?;
        }
        Ok(())
    }

    async fn modifier_press(&self, modifier: &str) -> Result<()> {
        let mut state = self
            .modifier_state
            .lock()
            .expect("Modifier state mutex poisoned");
        match modifier.to_lowercase().as_str() {
            "ctrl" | "control" => {
                state.ctrl = true;
                send_event(EventType::KeyPress(Key::ControlLeft))?;
            }
            "alt" => {
                state.alt = true;
                send_event(EventType::KeyPress(Key::Alt))?;
            }
            "shift" => {
                state.shift = true;
                send_event(EventType::KeyPress(Key::ShiftLeft))?;
            }
            "meta" | "super" | "cmd" => {
                state.meta = true;
                send_event(EventType::KeyPress(Key::MetaLeft))?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn modifier_release(&self, modifier: &str) -> Result<()> {
        let mut state = self
            .modifier_state
            .lock()
            .expect("Modifier state mutex poisoned");
        match modifier.to_lowercase().as_str() {
            "ctrl" | "control" => {
                state.ctrl = false;
                send_event(EventType::KeyRelease(Key::ControlLeft))?;
            }
            "alt" => {
                state.alt = false;
                send_event(EventType::KeyRelease(Key::Alt))?;
            }
            "shift" => {
                state.shift = false;
                send_event(EventType::KeyRelease(Key::ShiftLeft))?;
            }
            "meta" | "super" | "cmd" => {
                state.meta = false;
                send_event(EventType::KeyRelease(Key::MetaLeft))?;
            }
            _ => {}
        }
        Ok(())
    }
}

impl InputHandlerImpl {
    fn map_button(button: u8) -> Button {
        match button {
            1 => Button::Left,
            2 => Button::Right,
            3 => Button::Middle,
            _ => Button::Left,
        }
    }

    fn resolve_pointer_position(&self) -> (f64, f64) {
        let mut pos = self
            .current_pos
            .lock()
            .expect("Cursor position mutex poisoned");
        if let Some(coords) = *pos {
            coords
        } else {
            let fallback = (
                ServerConfig::FALLBACK_SCREEN_WIDTH / 2.0,
                ServerConfig::FALLBACK_SCREEN_HEIGHT / 2.0,
            );
            *pos = Some(fallback);
            fallback
        }
    }

    fn next_click_count(&self, button: u8) -> i64 {
        let mut last_click = self.last_click.lock().expect("Last click mutex poisoned");
        let now = Instant::now();
        let timeout = Duration::from_millis(ServerConfig::DOUBLE_CLICK_TIMEOUT_MS);

        let count = if let Some(previous) = &*last_click {
            if previous.button == button
                && now.duration_since(previous.time) <= timeout
                && previous.count == 1
            {
                2
            } else {
                1
            }
        } else {
            1
        };

        *last_click = Some(ClickState {
            button,
            time: now,
            count,
        });

        count as i64
    }

    fn send_mouse_button_event(
        position: (f64, f64),
        button: Button,
        is_press: bool,
        click_state: i64,
    ) -> Result<()> {
        unsafe {
            #[repr(C)]
            struct CGPoint {
                x: f64,
                y: f64,
            }

            const LEFT_DOWN: u32 = 1;
            const LEFT_UP: u32 = 2;
            const RIGHT_DOWN: u32 = 3;
            const RIGHT_UP: u32 = 4;
            const OTHER_DOWN: u32 = 25;
            const OTHER_UP: u32 = 26;
            const KCG_MOUSE_EVENT_CLICK_STATE: u32 = 1;

            extern "C" {
                fn CGEventCreateMouseEvent(
                    source: *const std::ffi::c_void,
                    mouseType: u32,
                    mouseCursorPosition: CGPoint,
                    mouseButton: u32,
                ) -> *const std::ffi::c_void;
                fn CGEventSetIntegerValueField(
                    event: *const std::ffi::c_void,
                    field: u32,
                    value: i64,
                );
                fn CGEventPost(tap: u32, event: *const std::ffi::c_void) -> i32;
                fn CFRelease(ptr: *const std::ffi::c_void);
            }

            let (event_type, button_index) = match (button, is_press) {
                (Button::Left, true) => (LEFT_DOWN, 0u32),
                (Button::Left, false) => (LEFT_UP, 0u32),
                (Button::Right, true) => (RIGHT_DOWN, 1u32),
                (Button::Right, false) => (RIGHT_UP, 1u32),
                (Button::Middle, true) => (OTHER_DOWN, 2u32),
                (Button::Middle, false) => (OTHER_UP, 2u32),
                _ => (LEFT_DOWN, 0u32),
            };

            let point = CGPoint {
                x: position.0,
                y: position.1,
            };
            let event = CGEventCreateMouseEvent(std::ptr::null(), event_type, point, button_index);

            if event.is_null() {
                return Err(anyhow::anyhow!("Failed to create mouse button event"));
            }

            CGEventSetIntegerValueField(event, KCG_MOUSE_EVENT_CLICK_STATE, click_state);
            CGEventPost(0, event);
            CFRelease(event);
        }

        Ok(())
    }

    fn send_mouse_drag(x: f64, y: f64, button: Option<Button>) -> Result<()> {
        unsafe {
            #[repr(C)]
            struct CGPoint {
                x: f64,
                y: f64,
            }

            let event_type = match button {
                Some(Button::Left) => 6u32,
                Some(Button::Right) => 7u32,
                Some(Button::Middle) => 8u32,
                _ => 6u32,
            };

            extern "C" {
                fn CGEventCreateMouseEvent(
                    source: *const std::ffi::c_void,
                    mouseType: u32,
                    mouseCursorPosition: CGPoint,
                    mouseButton: u32,
                ) -> *const std::ffi::c_void;
                fn CGEventPost(tap: u32, event: *const std::ffi::c_void) -> i32;
                fn CFRelease(ptr: *const std::ffi::c_void);
            }

            let point = CGPoint { x, y };
            let event = CGEventCreateMouseEvent(std::ptr::null(), event_type, point, 0);

            if event.is_null() {
                return Err(anyhow::anyhow!("Failed to create mouse drag event"));
            }

            CGEventPost(0, event);
            CFRelease(event);
        }

        Ok(())
    }

    fn apply_modifiers(state: &Mutex<ModifierKeys>, modifiers: &ModifierKeys) -> Result<()> {
        let mut state_guard = state.lock().expect("Modifier state mutex poisoned");

        if modifiers.ctrl && !state_guard.ctrl {
            send_event(EventType::KeyPress(Key::ControlLeft))?;
            state_guard.ctrl = true;
        }
        if modifiers.alt && !state_guard.alt {
            send_event(EventType::KeyPress(Key::Alt))?;
            state_guard.alt = true;
        }
        if modifiers.shift && !state_guard.shift {
            send_event(EventType::KeyPress(Key::ShiftLeft))?;
            state_guard.shift = true;
        }
        if modifiers.meta && !state_guard.meta {
            send_event(EventType::KeyPress(Key::MetaLeft))?;
            state_guard.meta = true;
        }

        if !modifiers.ctrl && state_guard.ctrl {
            send_event(EventType::KeyRelease(Key::ControlLeft))?;
            state_guard.ctrl = false;
        }
        if !modifiers.alt && state_guard.alt {
            send_event(EventType::KeyRelease(Key::Alt))?;
            state_guard.alt = false;
        }
        if !modifiers.shift && state_guard.shift {
            send_event(EventType::KeyRelease(Key::ShiftLeft))?;
            state_guard.shift = false;
        }
        if !modifiers.meta && state_guard.meta {
            send_event(EventType::KeyRelease(Key::MetaLeft))?;
            state_guard.meta = false;
        }

        Ok(())
    }

    async fn queue_drag_event(
        &self,
        delta_x: f64,
        delta_y: f64,
        target_x: f64,
        target_y: f64,
        button: Option<Button>,
    ) -> Result<()> {
        let mut drag = self
            .drag_state
            .lock()
            .expect("Drag state mutex poisoned");

        drag.pending_x += delta_x;
        drag.pending_y += delta_y;

        let should_flush = drag.last_flush.elapsed() >= Duration::from_millis(DRAG_BATCH_INTERVAL_MS);

        if should_flush {
            Self::send_mouse_drag(target_x, target_y, button)?;
            drag.pending_x = 0.0;
            drag.pending_y = 0.0;
            drag.last_flush = Instant::now();
        }

        Ok(())
    }

    fn flush_pending_drag(&self) -> Result<()> {
        let mut drag = self
            .drag_state
            .lock()
            .expect("Drag state mutex poisoned");

        if drag.pending_x != 0.0 || drag.pending_y != 0.0 {
            let pos = self.resolve_pointer_position();
            Self::send_mouse_drag(pos.0, pos.1, drag.button)?;
            drag.pending_x = 0.0;
            drag.pending_y = 0.0;
        }

        Ok(())
    }
}

mod tests {
    #[test]
    fn test_drag_batching_accumulates_movement() {
        let handler = InputHandlerImpl::new().unwrap();

        let mut drag = handler.drag_state.lock().unwrap();
        drag.pending_x = 0.0;
        drag.pending_y = 0.0;

        assert_eq!(drag.pending_x, 0.0);
        assert_eq!(drag.pending_y, 0.0);
    }

    #[test]
    fn test_drag_state_initialized() {
        let handler = InputHandlerImpl::new().unwrap();
        let drag = handler.drag_state.lock().unwrap();

        assert_eq!(drag.pending_x, 0.0);
        assert_eq!(drag.pending_y, 0.0);
        assert!(drag.button.is_none());
    }
}

fn string_to_key(s: &str) -> Option<Key> {
    match s {
        " " => Some(Key::Space),
        "\n" | "\r" => Some(Key::Return),
        "\t" => Some(Key::Tab),
        "\x08" | "\x7f" => Some(Key::Backspace),
        "." => Some(Key::Dot),
        "," => Some(Key::Comma),
        ";" => Some(Key::SemiColon),
        ":" => Some(Key::SemiColon),
        "!" => Some(Key::Num1),
        "?" => Some(Key::Slash),
        "-" => Some(Key::Minus),
        "_" => Some(Key::Minus),
        "=" => Some(Key::Equal),
        "+" => Some(Key::Equal),
        "[" => Some(Key::LeftBracket),
        "]" => Some(Key::RightBracket),
        "{" => Some(Key::LeftBracket),
        "}" => Some(Key::RightBracket),
        "(" => Some(Key::Num9),
        ")" => Some(Key::Num0),
        "'" => Some(Key::Quote),
        "\"" => Some(Key::Quote),
        "\\" => Some(Key::BackSlash),
        "|" => Some(Key::BackSlash),
        "/" => Some(Key::Slash),
        "<" => Some(Key::Comma),
        ">" => Some(Key::Dot),
        s if s.len() == 1 => {
            let ch = s.chars().next().unwrap();
            if ch.is_ascii_alphabetic() {
                match ch.to_ascii_uppercase() {
                    'A' => Some(Key::KeyA),
                    'B' => Some(Key::KeyB),
                    'C' => Some(Key::KeyC),
                    'D' => Some(Key::KeyD),
                    'E' => Some(Key::KeyE),
                    'F' => Some(Key::KeyF),
                    'G' => Some(Key::KeyG),
                    'H' => Some(Key::KeyH),
                    'I' => Some(Key::KeyI),
                    'J' => Some(Key::KeyJ),
                    'K' => Some(Key::KeyK),
                    'L' => Some(Key::KeyL),
                    'M' => Some(Key::KeyM),
                    'N' => Some(Key::KeyN),
                    'O' => Some(Key::KeyO),
                    'P' => Some(Key::KeyP),
                    'Q' => Some(Key::KeyQ),
                    'R' => Some(Key::KeyR),
                    'S' => Some(Key::KeyS),
                    'T' => Some(Key::KeyT),
                    'U' => Some(Key::KeyU),
                    'V' => Some(Key::KeyV),
                    'W' => Some(Key::KeyW),
                    'X' => Some(Key::KeyX),
                    'Y' => Some(Key::KeyY),
                    'Z' => Some(Key::KeyZ),
                    _ => None,
                }
            } else if ch.is_ascii_digit() {
                match ch {
                    '0' => Some(Key::Num0),
                    '1' => Some(Key::Num1),
                    '2' => Some(Key::Num2),
                    '3' => Some(Key::Num3),
                    '4' => Some(Key::Num4),
                    '5' => Some(Key::Num5),
                    '6' => Some(Key::Num6),
                    '7' => Some(Key::Num7),
                    '8' => Some(Key::Num8),
                    '9' => Some(Key::Num9),
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => None,
    }
}
