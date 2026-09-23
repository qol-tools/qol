//! Window geometry.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MonitorBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A window frame in screen coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl WindowRect {
    pub fn from_array([x, y, width, height]: [f64; 4]) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn to_array(self) -> [f64; 4] {
        [self.x, self.y, self.width, self.height]
    }

    /// Build a rect positioned at the origin from a size in f32
    /// (picker-style width/height pairs).
    pub fn from_size_f32(width: f32, height: f32) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: f64::from(width),
            height: f64::from(height),
        }
    }

    pub fn size_f32(self) -> (f32, f32) {
        (self.width as f32, self.height as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_round_trip() {
        let rect = WindowRect::from_array([1.5, -2.0, 800.0, 600.0]);
        assert_eq!(rect.to_array(), [1.5, -2.0, 800.0, 600.0]);
        assert_eq!(rect.x, 1.5);
        assert_eq!(rect.y, -2.0);
    }

    #[test]
    fn size_f32_round_trip() {
        let rect = WindowRect::from_size_f32(1920.0, 1080.0);
        assert_eq!(rect.size_f32(), (1920.0, 1080.0));
        assert_eq!((rect.x, rect.y), (0.0, 0.0));
    }

    #[test]
    fn default_is_zero_rect() {
        assert_eq!(WindowRect::default().to_array(), [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn monitor_bounds_serde_round_trip() {
        let bounds = MonitorBounds {
            x: -1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let json = serde_json::to_string(&bounds).unwrap();
        assert_eq!(
            serde_json::from_str::<MonitorBounds>(&json).unwrap(),
            bounds
        );
    }
}
