use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::glide::{Direction, Phase};

use super::scripts::EXCLUDED_WINDOW_TYPE_EXPRESSION;

const WATCHDOG_MS: u64 = 1000;

pub(crate) struct GlideController {
    session: qol_platform::cinnamon::Session,
    active: HashSet<Direction>,
    expires_at: Option<Instant>,
}

impl GlideController {
    pub(crate) fn connect() -> Result<Self, String> {
        qol_platform::cinnamon::Session::connect().map(|session| Self {
            session,
            active: HashSet::new(),
            expires_at: None,
        })
    }

    pub(crate) fn update(
        &mut self,
        direction: Direction,
        phase: Phase,
        speed: f64,
    ) -> Result<String, String> {
        match phase {
            Phase::Start => self.start(direction, speed),
            Phase::Heartbeat => self.heartbeat(speed),
            Phase::Stop => self.stop(direction),
        }
    }

    pub(crate) fn stop_all(&mut self) -> Result<(), String> {
        self.active.clear();
        self.expires_at = None;
        self.session.eval(STOP_ALL_SCRIPT).map(|_| ())
    }

    pub(crate) fn maintain(&mut self) -> Option<Result<(), String>> {
        let expires_at = self.expires_at?;
        if Instant::now() < expires_at {
            return None;
        }
        Some(self.stop_all())
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.active.is_empty()
    }

    fn start(&mut self, direction: Direction, speed: f64) -> Result<String, String> {
        let first_direction = self.active.is_empty();
        let result = self
            .session
            .eval(&start_script(direction, speed, WATCHDOG_MS));
        let observation = match result {
            Ok(observation) => observation,
            Err(error) => {
                if first_direction {
                    let _ = self.stop_all();
                }
                return Err(error);
            }
        };
        self.active.insert(direction);
        self.refresh_watchdog();
        Ok(observation)
    }

    fn heartbeat(&mut self, speed: f64) -> Result<String, String> {
        if !self.is_active() {
            return Ok("active=none native_move=inactive reason=no-active-glide".into());
        }
        match self.session.eval(&heartbeat_script(speed, WATCHDOG_MS)) {
            Ok(observation) => {
                self.refresh_watchdog();
                Ok(observation)
            }
            Err(error) => {
                let _ = self.stop_all();
                Err(error)
            }
        }
    }

    fn stop(&mut self, direction: Direction) -> Result<String, String> {
        let result = self.session.eval(&stop_script(direction));
        self.active.remove(&direction);
        if self.active.is_empty() {
            self.expires_at = None;
        }
        result
    }

    fn refresh_watchdog(&mut self) {
        self.expires_at = Some(Instant::now() + Duration::from_millis(WATCHDOG_MS));
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
    const Meta = imports.gi.Meta;
    const key = '__qolWindowActionsGlide';
    const now = Date.now();
    const endNativeMove = state => {{
        if (!state.ownsNativeMove) {{
            return;
        }}
        if (global.display.get_grab_op() === state.nativeGrabOp) {{
            global.display.end_grab_op(global.get_current_time());
        }}
        state.ownsNativeMove = false;
    }};
    const applyMotion = (state, elapsed) => {{
        const left = state.directions.left || 0;
        const right = state.directions.right || 0;
        const up = state.directions.up || 0;
        const down = state.directions.down || 0;
        let dx = right === left ? 0 : (right > left ? 1 : -1);
        let dy = down === up ? 0 : (down > up ? 1 : -1);
        if (dx !== 0 && dy !== 0) {{
            dx *= 0.70710678118;
            dy *= 0.70710678118;
        }}
        state.dx = dx;
        state.dy = dy;
        const nextX = state.x + dx * state.speed * elapsed;
        const nextY = state.y + dy * state.speed * elapsed;
        const targetX = Math.round(nextX);
        const targetY = Math.round(nextY);
        state.win.move_frame(true, targetX, targetY);
        const actual = state.win.get_frame_rect();
        state.x = actual.x === targetX ? nextX : actual.x;
        state.y = actual.y === targetY ? nextY : actual.y;
        const focused = global.display.focus_window;
        const focusXid = focused ? String(focused.get_xwindow()) : 'none';
        if (focusXid !== state.lastFocusXid) {{
            const [pointerX, pointerY] = global.get_pointer();
            const pointerInside = pointerX >= actual.x
                && pointerX < actual.x + actual.width
                && pointerY >= actual.y
                && pointerY < actual.y + actual.height;
            if (state.focusEvents.length === 16) {{
                state.focusEvents.shift();
            }}
            state.focusEvents.push(
                'at_ms=' + (Date.now() - state.startedAt)
                + ',from=' + state.lastFocusXid
                + ',to=' + focusXid
                + ',pointer=' + pointerX + ':' + pointerY
                + ',frame=' + actual.x + ':' + actual.y + ':'
                    + actual.width + ':' + actual.height
                + ',pointer_inside=' + pointerInside
                + ',vector=' + state.dx + ':' + state.dy
            );
            state.lastFocusXid = focusXid;
        }}
    }};
    (() => {{
        let state = global[key];
        if (!state) {{
            const win = global.display.focus_window;
            if (!win) {{
                return 'ERROR: No focused window';
            }}
            if ({EXCLUDED_WINDOW_TYPE_EXPRESSION}) {{
                return 'ERROR: Focused surface is not an app window';
            }}
            if (win.maximized_horizontally || win.maximized_vertically) {{
                win.unmaximize(3);
            }}
            if (global.display.get_grab_op() !== Meta.GrabOp.NONE) {{
                return 'ERROR: Another Cinnamon window operation is active';
            }}
            const rect = win.get_frame_rect();
            const targetXid = String(win.get_xwindow());
            const focusMode = ['click', 'sloppy', 'mouse'][Meta.prefs_get_focus_mode()]
                || 'unknown';
            const [grabX, grabY] = global.get_pointer();
            const nativeGrabOp = Meta.GrabOp.MOVING;
            const beganNativeMove = global.display.begin_grab_op(
                win,
                nativeGrabOp,
                false,
                true,
                0,
                0,
                global.get_current_time(),
                grabX,
                grabY
            );
            if (!beganNativeMove || global.display.get_grab_op() !== nativeGrabOp) {{
                if (global.display.get_grab_op() === nativeGrabOp) {{
                    global.display.end_grab_op(global.get_current_time());
                }}
                return 'ERROR: Cinnamon could not begin the native window move';
            }}
            state = {{
                win: win,
                targetXid: targetXid,
                focusMode: focusMode,
                nativeGrabOp: nativeGrabOp,
                ownsNativeMove: true,
                lastFocusXid: targetXid,
                startedAt: now,
                focusEvents: [],
                directions: {{}},
                sequence: 0,
                x: rect.x,
                y: rect.y,
                dx: 0,
                dy: 0,
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
                    endNativeMove(state);
                    delete global[key];
                    return GLib.SOURCE_REMOVE;
                }}
                const elapsed = Math.min((tick - state.lastTick) / 1000, 0.05);
                state.lastTick = tick;
                applyMotion(state, elapsed);
                return GLib.SOURCE_CONTINUE;
            }});
        }}
        state.sequence = (state.sequence || 0) + 1;
        state.directions.{direction} = state.sequence;
        state.speed = {speed};
        state.expiresAt = now + {watchdog_ms};
        applyMotion(state, 1 / 60);
        state.lastTick = now;
        const active = ['left', 'right', 'up', 'down']
            .filter(direction => state.directions[direction])
            .sort((a, b) => state.directions[a] - state.directions[b])
            .map(direction => direction + ':' + state.directions[direction])
            .join(',');
        const focused = global.display.focus_window;
        const focusXid = focused ? String(focused.get_xwindow()) : 'none';
        const frame = state.win.get_frame_rect();
        const [pointerX, pointerY] = global.get_pointer();
        const pointerInside = pointerX >= frame.x
            && pointerX < frame.x + frame.width
            && pointerY >= frame.y
            && pointerY < frame.y + frame.height;
        const focusEvents = state.focusEvents.splice(0).join(';') || 'none';
        return 'active=' + (active || 'none')
            + ' vector=' + state.dx + ',' + state.dy
            + ' position=' + Math.round(state.x) + ',' + Math.round(state.y)
            + ' target_xid=' + state.targetXid
            + ' focus_xid=' + focusXid
            + ' focus_mode=' + state.focusMode
            + ' native_move=' + (state.ownsNativeMove ? 'active' : 'inactive')
            + ' grab_op=' + global.display.get_grab_op()
            + ' pointer=' + pointerX + ',' + pointerY
            + ' frame=' + frame.x + ',' + frame.y + ','
                + frame.width + ',' + frame.height
            + ' pointer_inside=' + pointerInside
            + ' focus_events=' + focusEvents;
    }})()
"#,
        direction = direction.as_str(),
    )
}

fn heartbeat_script(speed: f64, watchdog_ms: u64) -> String {
    let speed = speed.clamp(100.0, 4000.0);
    format!(
        r#"
    (() => {{
        const state = global.__qolWindowActionsGlide;
        if (!state) {{
            return 'active=none focus_events=none';
        }}
        state.speed = {speed};
        state.expiresAt = Date.now() + {watchdog_ms};
        const focused = global.display.focus_window;
        const focusXid = focused ? String(focused.get_xwindow()) : 'none';
        const frame = state.win.get_frame_rect();
        const [pointerX, pointerY] = global.get_pointer();
        const pointerInside = pointerX >= frame.x
            && pointerX < frame.x + frame.width
            && pointerY >= frame.y
            && pointerY < frame.y + frame.height;
        const focusEvents = state.focusEvents.splice(0).join(';') || 'none';
        return 'active=heartbeat'
            + ' target_xid=' + state.targetXid
            + ' focus_xid=' + focusXid
            + ' focus_mode=' + state.focusMode
            + ' native_move=' + (state.ownsNativeMove ? 'active' : 'inactive')
            + ' grab_op=' + global.display.get_grab_op()
            + ' pointer=' + pointerX + ',' + pointerY
            + ' frame=' + frame.x + ',' + frame.y + ','
                + frame.width + ',' + frame.height
            + ' pointer_inside=' + pointerInside
            + ' focus_events=' + focusEvents;
    }})()
"#
    )
}

fn stop_script(direction: Direction) -> String {
    format!(
        r#"
    const GLib = imports.gi.GLib;
    const key = '__qolWindowActionsGlide';
    const endNativeMove = state => {{
        if (!state.ownsNativeMove) {{
            return;
        }}
        if (global.display.get_grab_op() === state.nativeGrabOp) {{
            global.display.end_grab_op(global.get_current_time());
        }}
        state.ownsNativeMove = false;
    }};
    (() => {{
        const state = global[key];
        if (!state) {{
            return 'active=none vector=0,0 position=unknown';
        }}
        delete state.directions.{direction};
        const left = state.directions.left || 0;
        const right = state.directions.right || 0;
        const up = state.directions.up || 0;
        const down = state.directions.down || 0;
        let dx = right === left ? 0 : (right > left ? 1 : -1);
        let dy = down === up ? 0 : (down > up ? 1 : -1);
        if (dx !== 0 && dy !== 0) {{
            dx *= 0.70710678118;
            dy *= 0.70710678118;
        }}
        const active = ['left', 'right', 'up', 'down']
            .filter(direction => state.directions[direction])
            .sort((a, b) => state.directions[a] - state.directions[b])
            .map(direction => direction + ':' + state.directions[direction])
            .join(',');
        const focused = global.display.focus_window;
        const focusXid = focused ? String(focused.get_xwindow()) : 'none';
        const frame = state.win.get_frame_rect();
        const [pointerX, pointerY] = global.get_pointer();
        const pointerInside = pointerX >= frame.x
            && pointerX < frame.x + frame.width
            && pointerY >= frame.y
            && pointerY < frame.y + frame.height;
        const focusEvents = state.focusEvents.splice(0).join(';') || 'none';
        if (!active) {{
            if (state.sourceId) {{
                GLib.source_remove(state.sourceId);
            }}
            endNativeMove(state);
            delete global[key];
        }}
        return 'active=' + (active || 'none')
            + ' vector=' + dx + ',' + dy
            + ' position=' + Math.round(state.x) + ',' + Math.round(state.y)
            + ' target_xid=' + state.targetXid
            + ' focus_xid=' + focusXid
            + ' focus_mode=' + state.focusMode
            + ' native_move=' + (active ? 'active' : 'released')
            + ' grab_op=' + global.display.get_grab_op()
            + ' pointer=' + pointerX + ',' + pointerY
            + ' frame=' + frame.x + ',' + frame.y + ','
                + frame.width + ',' + frame.height
            + ' pointer_inside=' + pointerInside
            + ' focus_events=' + focusEvents;
    }})()
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
        if (state.ownsNativeMove
            && global.display.get_grab_op() === state.nativeGrabOp) {
            global.display.end_grab_op(global.get_current_time());
        }
        state.ownsNativeMove = false;
        delete global[key];
    }
    'Glide stopped';
"#;

#[cfg(test)]
mod tests {
    use super::{
        heartbeat_script, start_script, stop_script, Direction, EXCLUDED_WINDOW_TYPE_EXPRESSION,
        WATCHDOG_MS,
    };

    #[test]
    fn glide_uses_render_rate_time_based_motion_and_watchdog() {
        let script = start_script(Direction::Left, 1200.0, WATCHDOG_MS);
        let required = [
            "timeout_add(GLib.PRIORITY_DEFAULT, 16",
            "(tick - state.lastTick) / 1000",
            "tick > state.expiresAt",
            "endNativeMove(state)",
            "delete global[key]",
            "move_frame(true",
            "applyMotion(state, 1 / 60)",
            "return 'active=' + (active || 'none')",
            "+ ' vector=' + state.dx + ',' + state.dy",
        ];
        for fragment in required {
            assert!(script.contains(fragment), "missing {fragment}\n{script}");
        }
    }

    #[test]
    fn glide_uses_cinnamons_native_move_without_warping_the_pointer() {
        let start = start_script(Direction::Right, 1200.0, WATCHDOG_MS);
        for fragment in [
            "global.display.begin_grab_op(",
            "Meta.GrabOp.MOVING",
            "const [grabX, grabY] = global.get_pointer()",
            "global.get_current_time(),\n                grabX,\n                grabY",
            "global.display.get_grab_op() !== nativeGrabOp",
            "native_move=' + (state.ownsNativeMove ? 'active' : 'inactive')",
        ] {
            assert!(start.contains(fragment), "missing {fragment}\n{start}");
        }
        assert!(!start.contains("win.begin_grab_op("));

        let stop = stop_script(Direction::Right);
        assert!(stop.contains("global.display.end_grab_op(global.get_current_time())"));
        assert!(stop.contains("native_move=' + (active ? 'active' : 'released')"));
    }

    #[test]
    fn glide_rejects_desktop_and_dock_surfaces_before_window_mutation() {
        let script = start_script(Direction::Right, 1200.0, WATCHDOG_MS);
        let guard = script.find(EXCLUDED_WINDOW_TYPE_EXPRESSION).unwrap();
        let mutation = script.find("win.unmaximize(3)").unwrap();

        assert!(guard < mutation, "{script}");
    }

    #[test]
    fn scripts_target_the_requested_direction_and_clamp_speed() {
        let left = start_script(Direction::Left, 10_000.0, WATCHDOG_MS);
        assert!(left.contains("state.directions.left = state.sequence"));
        assert!(left.contains("speed: 4000"));

        let stop = stop_script(Direction::Up);
        assert!(stop.contains("delete state.directions.up"));
        assert!(stop.contains("+ ' vector=' + dx + ',' + dy"));

        let heartbeat = heartbeat_script(1.0, WATCHDOG_MS);
        assert!(heartbeat.contains("state.speed = 100"));
    }

    #[test]
    fn most_recent_direction_wins_on_each_axis() {
        let script = start_script(Direction::Right, 1200.0, WATCHDOG_MS);
        let required = [
            "sequence: 0",
            "state.sequence = (state.sequence || 0) + 1",
            "right === left ? 0 : (right > left ? 1 : -1)",
            "down === up ? 0 : (down > up ? 1 : -1)",
        ];
        for fragment in required {
            assert!(script.contains(fragment), "missing {fragment}\n{script}");
        }
    }

    #[test]
    fn glide_resynchronizes_when_cinnamon_constrains_motion() {
        let script = start_script(Direction::Up, 1200.0, WATCHDOG_MS);
        let required = [
            "const targetX = Math.round(nextX)",
            "const targetY = Math.round(nextY)",
            "const actual = state.win.get_frame_rect()",
            "state.x = actual.x === targetX ? nextX : actual.x",
            "state.y = actual.y === targetY ? nextY : actual.y",
        ];
        for fragment in required {
            assert!(script.contains(fragment), "missing {fragment}\n{script}");
        }
    }

    #[test]
    fn glide_reports_focus_context_and_transitions() {
        let start = start_script(Direction::Right, 1200.0, WATCHDOG_MS);
        let heartbeat = heartbeat_script(1200.0, WATCHDOG_MS);
        let stop = stop_script(Direction::Right);
        for fragment in [
            "target_xid=",
            "focus_xid=",
            "focus_mode=",
            "pointer_inside=",
            "focus_events=",
        ] {
            assert!(start.contains(fragment), "missing {fragment}\n{start}");
            assert!(
                heartbeat.contains(fragment),
                "missing {fragment}\n{heartbeat}"
            );
            assert!(stop.contains(fragment), "missing {fragment}\n{stop}");
        }
        for fragment in ["at_ms=", ",from=", ",to=", ",frame=", ",vector="] {
            assert!(start.contains(fragment), "missing {fragment}\n{start}");
        }
    }
}
