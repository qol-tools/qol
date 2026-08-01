use std::time::Duration;

use ratatui::style::Color;

mod activity;
mod console_state;
mod dash;
mod disk;
mod doctor;
mod draw;
mod emu_panel;
mod feature_flags;
mod filters;
mod key_bindings;
mod log_pane;
mod picker;
mod reload;
mod render_util;
mod session;
mod stream_view;
#[cfg(test)]
mod testkit;
mod tray_handle;
mod worktrees_panel;

pub(crate) use session::{run_session, SessionEnd};
pub(crate) use tray_handle::{spawn_forwarders, TrayHandle};

use dash::{
    Dash, RebuildState, Reload, ReloadOutcome, ReloadProgress, TraceRate, TraceRenderer, View,
    WorktreeSelection,
};
use draw::frame_accent;
use emu_panel::emu_run_line;
use log_pane::{clamp_offset, window_start, LogPane, LogRing};
use render_util::ITEM_GAP;
use session::{copy_highlight, core_log_dir, strip_ansi};
use stream_view::draw_run_log;
use tray_handle::{terminate_child, try_wait};

const LOG_CAP: usize = 2000;
const TICK: Duration = Duration::from_millis(150);
const RELAXED_TRACE_INTERVAL: Duration = Duration::from_millis(300);
const HEALTH_PROBE_INTERVAL: Duration = Duration::from_secs(2);
const EMU_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const LINKS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const DOCTOR_BASE_INTERVAL: Duration = Duration::from_secs(10);
const DOCTOR_CAP_INTERVAL: Duration = Duration::from_secs(60);
const ENDPOINTS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const STOP_GRACE: Duration = Duration::from_secs(5);
const HANDOFF_STOP_INTERVAL: Duration = Duration::from_millis(50);
const SHADOW_READY_TIMEOUT: Duration = Duration::from_secs(20);
const SHADOW_READY_INTERVAL: Duration = Duration::from_millis(100);
const PROMOTION_TIMEOUT: Duration = Duration::from_secs(10);
const PROMOTION_INTERVAL: Duration = Duration::from_millis(100);
const CRASH_TAIL: usize = 40;
const ACK_TTL: Duration = Duration::from_secs(6);
const QUIT_CONFIRM_WINDOW: Duration = Duration::from_secs(3);
pub(super) const ORANGE: Color = Color::Rgb(255, 153, 0);
const BASE_ACCENT: Color = Color::Green;
