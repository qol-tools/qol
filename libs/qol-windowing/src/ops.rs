//! The window operation contract implemented by platform backends.

use crate::{WindowId, WindowRect};

/// Platform window operations shared by window consumers.
///
/// Consumers implement this trait once per platform backend. Every method
/// follows the same contract:
///
/// - `Ok(Some(value))` - the operation succeeded and produced a value.
/// - `Ok(None)` - the query succeeded, but the window is gone: the id no
///   longer refers to an existing window on this system. Only query methods
///   return this; the caller should treat the window as closed.
/// - `Err` - this backend cannot perform the operation: the operation is
///   unsupported or not implemented on the platform, the window id is not in
///   a form this backend handles, or the operation failed at runtime
///   (including acting on a window that no longer exists).
///
/// A backend that does not implement an operation must return a typed `Err`,
/// never an `Ok` that silently does nothing and never `Ok(None)` unless it
/// actually queried the system and found the window gone. Callers surface
/// `Err` to the user and use `Ok(None)` to drop a stale window from their
/// state.
///
/// Enumeration order is stacking order (first entry = topmost window) where
/// the platform provides it.
pub trait WindowOps {
    /// Enumerate windows in stacking order, topmost first.
    fn enumerate_windows(&self) -> Result<Vec<WindowId>, String>;

    /// Current geometry of a window.
    ///
    /// `Ok(None)` means the window no longer exists; `Err` means this backend
    /// cannot report geometry for the id.
    fn window_geometry(&self, window_id: &WindowId) -> Result<Option<WindowRect>, String>;

    /// Move and resize a window to the given frame.
    ///
    /// `Err` means the backend cannot move the window: unsupported, the
    /// window is gone, or the frame could not be applied.
    fn move_resize(&self, window_id: &WindowId, rect: WindowRect) -> Result<(), String>;

    /// Focus and raise a window. Returns `Ok(false)` when the window cannot
    /// be brought forward.
    fn focus_window(&self, window_id: &WindowId) -> Result<bool, String>;

    /// Minimize a window. Returns `Ok(false)` when the window could not be
    /// minimized.
    fn minimize_window(&self, window_id: &WindowId) -> Result<bool, String>;

    /// Restore a minimized window. Returns `Ok(false)` when the window could
    /// not be restored.
    fn restore_window(&self, window_id: &WindowId) -> Result<bool, String>;
}
