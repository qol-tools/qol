mod platform;

use crate::command::{Command, ModifierKeys};
use crate::config::ServerConfig;
use anyhow::Result;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use platform::InputHandlerImpl;

const MAX_POINTER_DELTA_PIXELS: f64 = 4096.0;
const MAX_SCROLL_DELTA_NOTCHES: f64 = 64.0;
const SCREEN_BOUNDS_REFRESH: Duration = Duration::from_secs(5);

pub struct InputHandler {
    inner: InputHandlerImpl,
}

pub(crate) struct PlatformSupport {
    pub name: &'static str,
    pub declared: bool,
    pub input_backend: bool,
}

pub(crate) struct InputReadiness {
    pub platform: &'static str,
    pub ready: bool,
    pub authorization_granted: Option<bool>,
    pub display_env_set: Option<bool>,
    pub backend: &'static str,
    pub issue: Option<String>,
}

impl InputHandler {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: InputHandlerImpl::new()?,
        })
    }

    pub fn handle_command(&self, command: Command) -> Result<()> {
        match command {
            Command::MouseMove { x, y } => {
                let (x, y) = bounded_pair("MouseMove", x, y, MAX_POINTER_DELTA_PIXELS)?;
                self.inner.mouse_move(x, y)
            }
            Command::MouseClick { button } => self.inner.mouse_click(button),
            Command::MouseDown { button } => self.inner.mouse_down(button),
            Command::MouseUp { button } => self.inner.mouse_up(button),
            Command::MouseScroll { delta_x, delta_y } => {
                let (delta_x, delta_y) =
                    bounded_pair("MouseScroll", delta_x, delta_y, MAX_SCROLL_DELTA_NOTCHES)?;
                self.inner.mouse_scroll(delta_x, delta_y)
            }
            Command::KeyPress { key, modifiers } => self.inner.key_press(&key, &modifiers),
            Command::KeyRelease { key, modifiers } => self.inner.key_release(&key, &modifiers),
            Command::ModifierPress { modifier } => self.inner.modifier_press(&modifier),
            Command::ModifierRelease { modifier } => self.inner.modifier_release(&modifier),
        }
    }
}

pub(crate) fn platform_support() -> PlatformSupport {
    platform::platform_support()
}

pub(crate) fn inspect_readiness() -> InputReadiness {
    platform::inspect_readiness()
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

fn bounded_pair(command: &str, x: f64, y: f64, limit: f64) -> Result<(f64, f64)> {
    if !x.is_finite() || !y.is_finite() {
        anyhow::bail!("{command} carried a non-finite value");
    }
    Ok((x.clamp(-limit, limit), y.clamp(-limit, limit)))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::input) struct ScreenBounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl ScreenBounds {
    pub(in crate::input) fn from_origin_and_size(
        origin_x: f64,
        origin_y: f64,
        width: f64,
        height: f64,
    ) -> Option<Self> {
        let sized = width >= 1.0 && height >= 1.0;
        let finite = [origin_x, origin_y, width, height]
            .into_iter()
            .all(f64::is_finite);
        (sized && finite).then_some(Self {
            min_x: origin_x,
            min_y: origin_y,
            max_x: origin_x + width - 1.0,
            max_y: origin_y + height - 1.0,
        })
    }

    pub(in crate::input) fn assumed_desktop() -> Self {
        Self {
            min_x: 0.0,
            min_y: 0.0,
            max_x: ServerConfig::FALLBACK_SCREEN_WIDTH - 1.0,
            max_y: ServerConfig::FALLBACK_SCREEN_HEIGHT - 1.0,
        }
    }

    pub(in crate::input) fn center(&self) -> (f64, f64) {
        (
            (self.min_x + self.max_x) / 2.0,
            (self.min_y + self.max_y) / 2.0,
        )
    }

    pub(in crate::input) fn clamp(&self, x: f64, y: f64) -> (f64, f64) {
        (
            x.clamp(self.min_x, self.max_x),
            y.clamp(self.min_y, self.max_y),
        )
    }
}

pub(in crate::input) struct ScreenBoundsCache {
    state: Mutex<(ScreenBounds, Instant)>,
    query: fn() -> Option<ScreenBounds>,
}

impl ScreenBoundsCache {
    pub(in crate::input) fn new(query: fn() -> Option<ScreenBounds>) -> Self {
        Self {
            state: Mutex::new((
                query().unwrap_or_else(ScreenBounds::assumed_desktop),
                Instant::now(),
            )),
            query,
        }
    }

    pub(in crate::input) fn current(&self) -> ScreenBounds {
        let mut state = self.state.lock().expect("Screen bounds mutex poisoned");
        if state.1.elapsed() >= SCREEN_BOUNDS_REFRESH {
            if let Some(refreshed) = (self.query)() {
                state.0 = refreshed;
            }
            state.1 = Instant::now();
        }
        state.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn non_finite_pointer_values_are_rejected() {
        for (x, y) in [
            (f64::NAN, 0.0),
            (0.0, f64::NAN),
            (f64::INFINITY, 0.0),
            (0.0, f64::NEG_INFINITY),
        ] {
            assert!(bounded_pair("MouseMove", x, y, MAX_POINTER_DELTA_PIXELS).is_err());
        }
    }

    #[test]
    fn oversized_deltas_are_clamped_to_their_limit() {
        let (x, y) = bounded_pair("MouseMove", f64::MAX, -f64::MAX, MAX_POINTER_DELTA_PIXELS)
            .expect("finite values are accepted");
        let (scroll_x, scroll_y) =
            bounded_pair("MouseScroll", 200_000.0, -1e9, MAX_SCROLL_DELTA_NOTCHES)
                .expect("finite values are accepted");

        assert_eq!(
            (x, y),
            (MAX_POINTER_DELTA_PIXELS, -MAX_POINTER_DELTA_PIXELS)
        );
        assert_eq!(
            (scroll_x, scroll_y),
            (MAX_SCROLL_DELTA_NOTCHES, -MAX_SCROLL_DELTA_NOTCHES)
        );
    }

    #[test]
    fn ordinary_deltas_pass_through_unchanged() {
        let (x, y) = bounded_pair("MouseMove", 12.5, -3.25, MAX_POINTER_DELTA_PIXELS).unwrap();

        assert_eq!((x, y), (12.5, -3.25));
    }

    #[test]
    fn bounds_clamp_positions_into_the_visible_desktop() {
        let bounds = ScreenBounds::from_origin_and_size(0.0, 0.0, 1920.0, 1080.0).unwrap();

        assert_eq!(bounds.clamp(1e300, 1e300), (1919.0, 1079.0));
        assert_eq!(bounds.clamp(-1e300, -1e300), (0.0, 0.0));
        assert_eq!(bounds.clamp(640.0, 480.0), (640.0, 480.0));
        assert_eq!(bounds.center(), (959.5, 539.5));
    }

    #[test]
    fn bounds_keep_desktops_that_start_left_of_the_origin() {
        let spanned = ScreenBounds::from_origin_and_size(-1280.0, -200.0, 3200.0, 1280.0).unwrap();

        assert_eq!(spanned.clamp(-5000.0, -5000.0), (-1280.0, -200.0));
        assert_eq!(spanned.clamp(5000.0, 5000.0), (1919.0, 1079.0));
    }

    #[test]
    fn unusable_sizes_have_no_bounds() {
        assert!(ScreenBounds::from_origin_and_size(0.0, 0.0, 0.0, 1080.0).is_none());
        assert!(ScreenBounds::from_origin_and_size(0.0, 0.0, 1920.0, 0.0).is_none());
        assert!(ScreenBounds::from_origin_and_size(0.0, 0.0, f64::NAN, 1080.0).is_none());
    }

    #[test]
    fn cached_bounds_are_queried_once_inside_the_refresh_window() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn query() -> Option<ScreenBounds> {
            CALLS.fetch_add(1, Ordering::SeqCst);
            ScreenBounds::from_origin_and_size(0.0, 0.0, 1920.0, 1080.0)
        }

        let cache = ScreenBoundsCache::new(query);

        assert_eq!(cache.current(), cache.current());
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cached_bounds_fall_back_when_the_platform_cannot_answer() {
        fn query() -> Option<ScreenBounds> {
            None
        }

        let cache = ScreenBoundsCache::new(query);

        assert_eq!(cache.current(), ScreenBounds::assumed_desktop());
    }
}
