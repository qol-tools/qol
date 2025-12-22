use anyhow::Result;
use crate::input::InputHandlerTrait;
use crate::domain::config::ServerConfig;
use crate::domain::models::ModifierKeys;
use rdev::{simulate, Button, Key, EventType, SimulateError};
use std::time::Duration;
use std::sync::Mutex;

#[cfg(target_os = "linux")]
use x11::xlib;

pub struct InputHandlerImpl {
    current_pos: Mutex<Option<(f64, f64)>>,
    modifier_state: Mutex<ModifierKeys>,
}

impl InputHandlerImpl {
    pub fn new() -> Result<Self> {
        Ok(Self {
            current_pos: Mutex::new(None),
            modifier_state: Mutex::new(ModifierKeys::default()),
        })
    }

    fn get_cursor_position() -> Option<(f64, f64)> {
        unsafe {
            let display = xlib::XOpenDisplay(std::ptr::null());
            if display.is_null() {
                return None;
            }

            let mut root = 0;
            let mut child = 0;
            let mut root_x = 0;
            let mut root_y = 0;
            let mut win_x = 0;
            let mut win_y = 0;
            let mut mask = 0;

            xlib::XQueryPointer(
                display,
                xlib::XRootWindow(display, xlib::XDefaultScreen(display)),
                &mut root,
                &mut child,
                &mut root_x,
                &mut root_y,
                &mut win_x,
                &mut win_y,
                &mut mask,
            );

            xlib::XCloseDisplay(display);
            Some((root_x as f64, root_y as f64))
        }
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
        let mut pos_opt = self.current_pos.lock()
            .expect("Cursor position mutex poisoned");
        
        let (new_x, new_y) = if let Some((px, py)) = *pos_opt {
            (px + x, py + y)
        } else if let Some((cx, cy)) = Self::get_cursor_position() {
            (cx + x, cy + y)
        } else {
            (ServerConfig::FALLBACK_SCREEN_WIDTH / 2.0 + x,
             ServerConfig::FALLBACK_SCREEN_HEIGHT / 2.0 + y)
        };
        
        *pos_opt = Some((new_x, new_y));
        
        send_event(EventType::MouseMove {
            x: new_x,
            y: new_y,
        })?;
        Ok(())
    }
    
    async fn mouse_click(&self, button: u8) -> Result<()> {
        let button_enum = match button {
            1 => Button::Left,
            2 => Button::Right,
            3 => Button::Middle,
            _ => Button::Left,
        };
        
        send_event(EventType::ButtonPress(button_enum))?;
        tokio::time::sleep(Duration::from_millis(ServerConfig::MOUSE_CLICK_DELAY_MS)).await;
        send_event(EventType::ButtonRelease(button_enum))?;
        Ok(())
    }
    
    async fn mouse_down(&self, button: u8) -> Result<()> {
        let button_enum = match button {
            1 => Button::Left,
            2 => Button::Right,
            3 => Button::Middle,
            _ => Button::Left,
        };
        
        send_event(EventType::ButtonPress(button_enum))?;
        Ok(())
    }
    
    async fn mouse_up(&self, button: u8) -> Result<()> {
        let button_enum = match button {
            1 => Button::Left,
            2 => Button::Right,
            3 => Button::Middle,
            _ => Button::Left,
        };
        
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
        let mut state = self.modifier_state.lock()
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
        let mut state = self.modifier_state.lock()
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
    fn apply_modifiers(state: &Mutex<ModifierKeys>, modifiers: &ModifierKeys) -> Result<()> {
        let mut state_guard = state.lock()
            .expect("Modifier state mutex poisoned");
        
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
