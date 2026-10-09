use anyhow::Result;

use crate::config::Config;
use crate::cursor::platform::shake::{self, CursorSession, ShakeBackend};
use crate::cursor::CursorEffect;

use super::game_focus::GameFocusDetector;

pub fn create_effect() -> Box<dyn CursorEffect> {
    shake::create_effect(LinuxBackend)
}

struct LinuxBackend;

impl ShakeBackend for LinuxBackend {
    type Session = Session;
    type Focus = GameFocusDetector;

    fn open_session(&self, config: &Config) -> Result<Session> {
        super::display::ensure_cursor_support()?;
        log::info!("started mode=tree");
        Ok(Session::Tree(super::display::x11::CursorSession::open(
            config.scale_factor,
        )?))
    }

    fn open_focus(&self) -> Result<GameFocusDetector> {
        GameFocusDetector::open()
    }
}

enum Session {
    Tree(super::display::x11::CursorSession),
}

impl CursorSession for Session {
    fn set_scale(&mut self, scale: f32) -> bool {
        match self {
            Self::Tree(session) => session.set_scale(scale),
        }
    }

    fn refresh(&mut self) -> bool {
        match self {
            Self::Tree(session) => session.refresh(),
        }
    }

    fn restore(&mut self) {
        match self {
            Self::Tree(session) => session.restore(),
        }
    }

    fn live_cursor_hidden(&mut self) -> bool {
        match self {
            Self::Tree(session) => session.live_cursor_hidden(),
        }
    }
}
