use std::collections::HashSet;

use crate::movement::{Direction, Phase};

const WATCHDOG_MS: u64 = 1000;

pub(crate) struct GlideController {
    session: qol_platform::cinnamon::Session,
    active: HashSet<Direction>,
}

impl GlideController {
    pub(crate) fn connect() -> Result<Self, String> {
        qol_platform::cinnamon::Session::connect().map(|session| Self {
            session,
            active: HashSet::new(),
        })
    }

    pub(crate) fn update(
        &mut self,
        direction: Direction,
        phase: Phase,
        speed: f64,
    ) -> Result<(), String> {
        let script = match phase {
            Phase::Start => start_script(direction, speed, WATCHDOG_MS),
            Phase::Heartbeat => heartbeat_script(speed, WATCHDOG_MS),
            Phase::Stop => stop_script(direction),
        };
        self.session.eval(&script)?;
        match phase {
            Phase::Start => {
                self.active.insert(direction);
            }
            Phase::Heartbeat => {}
            Phase::Stop => {
                self.active.remove(&direction);
            }
        }
        Ok(())
    }

    pub(crate) fn stop_all(&mut self) -> Result<(), String> {
        self.active.clear();
        self.session.eval(STOP_ALL_SCRIPT)?;
        Ok(())
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.active.is_empty()
    }
}

impl Drop for GlideController {
    fn drop(&mut self) {
        if self.is_active() {
            let _ = self.stop_all();
        }
    }
}

fn start_script(direction: Direction, speed: f64, watchdog_ms: u64) -> String {
    let speed = speed.clamp(100.0, 4000.0);
    format!(
        r#"
    const GLib = imports.gi.GLib;
    const key = '__qolWindowActionsGlide';
    const now = Date.now();
    let state = global[key];
    if (!state) {{
        const win = global.display.focus_window;
        if (!win) {{
            'ERROR: No focused window';
        }} else {{
            if (win.maximized_horizontally || win.maximized_vertically) {{
                win.unmaximize(3);
            }}
            const rect = win.get_frame_rect();
            state = {{
                win: win,
                directions: {{}},
                x: rect.x,
                y: rect.y,
                speed: {speed},
                lastTick: now,
                expiresAt: now + {watchdog_ms},
                sourceId: 0
            }};
            global[key] = state;
            state.sourceId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 16, () => {{
                const active = global[key];
                if (!active || active !== state) {{
                    return GLib.SOURCE_REMOVE;
                }}
                const tick = Date.now();
                if (tick > state.expiresAt) {{
                    delete global[key];
                    return GLib.SOURCE_REMOVE;
                }}
                const elapsed = Math.min((tick - state.lastTick) / 1000, 0.05);
                state.lastTick = tick;
                let dx = (state.directions.right ? 1 : 0) - (state.directions.left ? 1 : 0);
                let dy = (state.directions.down ? 1 : 0) - (state.directions.up ? 1 : 0);
                if (dx !== 0 && dy !== 0) {{
                    dx *= 0.70710678118;
                    dy *= 0.70710678118;
                }}
                state.x += dx * state.speed * elapsed;
                state.y += dy * state.speed * elapsed;
                state.win.move_frame(true, Math.round(state.x), Math.round(state.y));
                return GLib.SOURCE_CONTINUE;
            }});
            state.directions.{direction} = true;
            'Glide started';
        }}
    }} else {{
        state.directions.{direction} = true;
        state.speed = {speed};
        state.expiresAt = now + {watchdog_ms};
        'Glide updated';
    }}
"#,
        direction = direction.as_str(),
    )
}

fn heartbeat_script(speed: f64, watchdog_ms: u64) -> String {
    let speed = speed.clamp(100.0, 4000.0);
    format!(
        r#"
    const state = global.__qolWindowActionsGlide;
    if (state) {{
        state.speed = {speed};
        state.expiresAt = Date.now() + {watchdog_ms};
    }}
    'Glide heartbeat';
"#
    )
}

fn stop_script(direction: Direction) -> String {
    format!(
        r#"
    const GLib = imports.gi.GLib;
    const key = '__qolWindowActionsGlide';
    const state = global[key];
    if (state) {{
        delete state.directions.{direction};
        if (Object.keys(state.directions).length === 0) {{
            if (state.sourceId) {{
                GLib.source_remove(state.sourceId);
            }}
            delete global[key];
        }}
    }}
    'Glide stopped';
"#,
        direction = direction.as_str(),
    )
}

const STOP_ALL_SCRIPT: &str = r#"
    const GLib = imports.gi.GLib;
    const key = '__qolWindowActionsGlide';
    const state = global[key];
    if (state) {
        if (state.sourceId) {
            GLib.source_remove(state.sourceId);
        }
        delete global[key];
    }
    'Glide stopped';
"#;

#[cfg(test)]
mod tests {
    use super::{heartbeat_script, start_script, stop_script, Direction, WATCHDOG_MS};

    #[test]
    fn glide_uses_render_rate_time_based_motion_and_watchdog() {
        let script = start_script(Direction::Left, 1200.0, WATCHDOG_MS);
        let required = [
            "timeout_add(GLib.PRIORITY_DEFAULT, 16",
            "(tick - state.lastTick) / 1000",
            "tick > state.expiresAt",
            "delete global[key]",
            "move_frame(true",
        ];
        for fragment in required {
            assert!(script.contains(fragment), "missing {fragment}\n{script}");
        }
    }

    #[test]
    fn scripts_target_the_requested_direction_and_clamp_speed() {
        let left = start_script(Direction::Left, 10_000.0, WATCHDOG_MS);
        assert!(left.contains("state.directions.left = true"));
        assert!(left.contains("speed: 4000"));

        let stop = stop_script(Direction::Up);
        assert!(stop.contains("delete state.directions.up"));

        let heartbeat = heartbeat_script(1.0, WATCHDOG_MS);
        assert!(heartbeat.contains("state.speed = 100"));
    }
}
