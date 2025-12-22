use anyhow::Result;
use crate::input::InputHandlerTrait;
use crate::domain::models::ModifierKeys;
use crate::domain::config::ServerConfig;
use std::sync::Mutex;
use std::time::Duration;
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};
use windows::{
    Win32::Foundation::POINT,
    Win32::UI::Input::KeyboardAndMouse::*,
};

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
            let mut point = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut point).is_ok() {
                Some((point.x as f64, point.y as f64))
            } else {
                None
            }
        }
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
        
        unsafe {
            SetCursorPos(new_x as i32, new_y as i32)?;
        }
        Ok(())
    }
    
    async fn mouse_click(&self, button: u8) -> Result<()> {
        self.mouse_down(button).await?;
        tokio::time::sleep(Duration::from_millis(ServerConfig::MOUSE_CLICK_DELAY_MS)).await;
        self.mouse_up(button).await?;
        Ok(())
    }
    
    async fn mouse_down(&self, button: u8) -> Result<()> {
        let flags = match button {
            1 => MOUSEEVENTF_LEFTDOWN,
            2 => MOUSEEVENTF_RIGHTDOWN,
            3 => MOUSEEVENTF_MIDDLEDOWN,
            _ => MOUSEEVENTF_LEFTDOWN,
        };
        
        unsafe {
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: 0,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        }
        Ok(())
    }
    
    async fn mouse_up(&self, button: u8) -> Result<()> {
        let flags = match button {
            1 => MOUSEEVENTF_LEFTUP,
            2 => MOUSEEVENTF_RIGHTUP,
            3 => MOUSEEVENTF_MIDDLEUP,
            _ => MOUSEEVENTF_LEFTUP,
        };
        
        unsafe {
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: 0,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        }
        Ok(())
    }
    
    async fn mouse_scroll(&self, delta_x: f64, delta_y: f64) -> Result<()> {
        unsafe {
            if delta_y != 0.0 {
                let input = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: 0,
                            dy: 0,
                            mouseData: (delta_y * 120.0) as i32 as u32,
                            dwFlags: MOUSEEVENTF_WHEEL,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            if delta_x != 0.0 {
                let input = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: 0,
                            dy: 0,
                            mouseData: (delta_x * 120.0) as i32 as u32,
                            dwFlags: MOUSEEVENTF_HWHEEL,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
        }
        Ok(())
    }
    
    async fn key_press(&self, key: &str, modifiers: &ModifierKeys) -> Result<()> {
        Self::apply_modifiers(&self.modifier_state, modifiers)?;
        
        if let Some(vk_code) = string_to_vk(key) {
            unsafe {
                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(vk_code),
                            wScan: 0,
                            dwFlags: KEYBD_EVENT_FLAGS(0u32),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
        }
        Ok(())
    }
    
    async fn key_release(&self, key: &str, _modifiers: &ModifierKeys) -> Result<()> {
        if let Some(vk_code) = string_to_vk(key) {
            unsafe {
                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(vk_code),
                            wScan: 0,
                            dwFlags: KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
        }
        Ok(())
    }
    
    async fn modifier_press(&self, modifier: &str) -> Result<()> {
        let mut state = self.modifier_state.lock()
            .expect("Modifier state mutex poisoned");
        match modifier.to_lowercase().as_str() {
            "ctrl" | "control" => {
                state.ctrl = true;
                unsafe {
                    let input = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VK_CONTROL,
                                wScan: 0,
                                dwFlags: KEYBD_EVENT_FLAGS(0u32),
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    };
                    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
                }
            }
            "alt" => {
                state.alt = true;
                unsafe {
                    let input = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VK_MENU,
                                wScan: 0,
                                dwFlags: KEYBD_EVENT_FLAGS(0u32),
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    };
                    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
                }
            }
            "shift" => {
                state.shift = true;
                unsafe {
                    let input = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VK_SHIFT,
                                wScan: 0,
                                dwFlags: KEYBD_EVENT_FLAGS(0u32),
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    };
                    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
                }
            }
            "meta" | "super" | "cmd" => {
                state.meta = true;
                unsafe {
                    let input = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VK_LWIN,
                                wScan: 0,
                                dwFlags: KEYBD_EVENT_FLAGS(0u32),
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    };
                    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
                }
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
                unsafe {
                    let input = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VK_CONTROL,
                                wScan: 0,
                                dwFlags: KEYEVENTF_KEYUP,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    };
                    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
                }
            }
            "alt" => {
                state.alt = false;
                unsafe {
                    let input = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VK_MENU,
                                wScan: 0,
                                dwFlags: KEYEVENTF_KEYUP,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    };
                    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
                }
            }
            "shift" => {
                state.shift = false;
                unsafe {
                    let input = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VK_SHIFT,
                                wScan: 0,
                                dwFlags: KEYEVENTF_KEYUP,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    };
                    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
                }
            }
            "meta" | "super" | "cmd" => {
                state.meta = false;
                unsafe {
                    let input = INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VK_LWIN,
                                wScan: 0,
                                dwFlags: KEYEVENTF_KEYUP,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    };
                    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
                }
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
            unsafe {
                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_CONTROL,
                            wScan: 0,
                            dwFlags: KEYBD_EVENT_FLAGS(0u32),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            state_guard.ctrl = true;
        }
        if modifiers.alt && !state_guard.alt {
            unsafe {
                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_MENU,
                            wScan: 0,
                            dwFlags: KEYBD_EVENT_FLAGS(0u32),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            state_guard.alt = true;
        }
        if modifiers.shift && !state_guard.shift {
            unsafe {
                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_SHIFT,
                            wScan: 0,
                            dwFlags: KEYBD_EVENT_FLAGS(0u32),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            state_guard.shift = true;
        }
        if modifiers.meta && !state_guard.meta {
            unsafe {
                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_LWIN,
                            wScan: 0,
                            dwFlags: KEYBD_EVENT_FLAGS(0u32),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            state_guard.meta = true;
        }
        
        if !modifiers.ctrl && state_guard.ctrl {
            unsafe {
                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_CONTROL,
                            wScan: 0,
                            dwFlags: KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            state_guard.ctrl = false;
        }
        if !modifiers.alt && state_guard.alt {
            unsafe {
                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_MENU,
                            wScan: 0,
                            dwFlags: KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            state_guard.alt = false;
        }
        if !modifiers.shift && state_guard.shift {
            unsafe {
                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_SHIFT,
                            wScan: 0,
                            dwFlags: KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            state_guard.shift = false;
        }
        if !modifiers.meta && state_guard.meta {
            unsafe {
                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_LWIN,
                            wScan: 0,
                            dwFlags: KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
            state_guard.meta = false;
        }
        
        Ok(())
    }
}

fn string_to_vk(s: &str) -> Option<u16> {
    match s {
        " " => Some(VK_SPACE.0),
        "\n" | "\r" => Some(VK_RETURN.0),
        "\t" => Some(VK_TAB.0),
        "\x08" | "\x7f" => Some(VK_BACK.0),
        "." => Some(VK_OEM_PERIOD.0),
        "," => Some(VK_OEM_COMMA.0),
        ";" => Some(VK_OEM_1.0),
        ":" => Some(VK_OEM_1.0),
        "!" => Some(VK_1.0),
        "?" => Some(VK_OEM_2.0),
        "-" => Some(VK_OEM_MINUS.0),
        "_" => Some(VK_OEM_MINUS.0),
        "=" => Some(VK_OEM_PLUS.0),
        "+" => Some(VK_OEM_PLUS.0),
        "[" => Some(VK_OEM_4.0),
        "]" => Some(VK_OEM_6.0),
        "{" => Some(VK_OEM_4.0),
        "}" => Some(VK_OEM_6.0),
        "(" => Some(VK_9.0),
        ")" => Some(VK_0.0),
        "'" => Some(VK_OEM_7.0),
        "\"" => Some(VK_OEM_7.0),
        "\\" => Some(VK_OEM_5.0),
        "|" => Some(VK_OEM_5.0),
        "/" => Some(VK_OEM_2.0),
        "<" => Some(VK_OEM_COMMA.0),
        ">" => Some(VK_OEM_PERIOD.0),
        s if s.len() == 1 => {
            let ch = s.chars().next().unwrap();
            if ch.is_ascii_alphabetic() {
                match ch.to_ascii_uppercase() {
                    'A' => Some(VK_A.0),
                    'B' => Some(VK_B.0),
                    'C' => Some(VK_C.0),
                    'D' => Some(VK_D.0),
                    'E' => Some(VK_E.0),
                    'F' => Some(VK_F.0),
                    'G' => Some(VK_G.0),
                    'H' => Some(VK_H.0),
                    'I' => Some(VK_I.0),
                    'J' => Some(VK_J.0),
                    'K' => Some(VK_K.0),
                    'L' => Some(VK_L.0),
                    'M' => Some(VK_M.0),
                    'N' => Some(VK_N.0),
                    'O' => Some(VK_O.0),
                    'P' => Some(VK_P.0),
                    'Q' => Some(VK_Q.0),
                    'R' => Some(VK_R.0),
                    'S' => Some(VK_S.0),
                    'T' => Some(VK_T.0),
                    'U' => Some(VK_U.0),
                    'V' => Some(VK_V.0),
                    'W' => Some(VK_W.0),
                    'X' => Some(VK_X.0),
                    'Y' => Some(VK_Y.0),
                    'Z' => Some(VK_Z.0),
                    _ => None,
                }
            } else if ch.is_ascii_digit() {
                match ch {
                    '0' => Some(VK_0.0),
                    '1' => Some(VK_1.0),
                    '2' => Some(VK_2.0),
                    '3' => Some(VK_3.0),
                    '4' => Some(VK_4.0),
                    '5' => Some(VK_5.0),
                    '6' => Some(VK_6.0),
                    '7' => Some(VK_7.0),
                    '8' => Some(VK_8.0),
                    '9' => Some(VK_9.0),
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

