use std::process::Command;

use crate::restore::WindowSystem;

const LAUNCHER_MATCH_MARKERS: [&str; 4] = [
    "qol-tray-launcher",
    "plugin-launcher",
    "qol-launcher",
    "qol launcher",
];

pub struct MacWindowSystem;

impl WindowSystem for MacWindowSystem {
    fn active_window_id(&self) -> Result<Option<String>, String> {
        // NSWorkspace is more reliable than processes.whose({frontmost:true}) which can race
        let script = r#"ObjC.import('AppKit'); const a=$.NSWorkspace.sharedWorkspace.frontmostApplication; a?String(a.processIdentifier):'0'"#;
        let out = run_jxa(script)?;
        let pid = out.trim();
        if pid == "0" || pid == "-1" || pid.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!("pid:{pid}:0")))
    }

    fn minimize_window(&self, window_id: &str) -> Result<bool, String> {
        let pid = parse_pid(window_id).ok_or_else(|| format!("Invalid window ID: {window_id}"))?;
        // Setting miniaturized=true via property fails with -10006 for most apps.
        // Clicking the AXMinimizeButton is the reliable alternative.
        let script = format!(
            "const proc=Application('System Events').processes.whose({{unixId:{}}})[0];\
             proc.windows[0].buttons.whose({{subrole:'AXMinimizeButton'}})[0].click();true",
            pid
        );
        run_jxa(&script)?;
        Ok(true)
    }

    fn stacking_window_ids(&self) -> Result<Vec<String>, String> {
        let script = r#"
            const procs = Application('System Events').processes.whose({visible: true});
            const ids = [];
            for (let i = 0; i < procs.length; i++) {
                try {
                    const proc = procs[i];
                    const pid = proc.unixId();
                    const wins = proc.windows;
                    for (let j = 0; j < wins.length; j++) {
                        try { if (wins[j].miniaturized()) ids.push('pid:' + pid + ':' + j); } catch(e) {}
                    }
                } catch(e) {}
            }
            ids.join(',')
        "#;
        let out = run_jxa(script)?;
        let out = out.trim();
        if out.is_empty() {
            return Ok(vec![]);
        }
        Ok(out.split(',').map(String::from).collect())
    }

    fn is_window_id(&self, id: &str) -> bool {
        id.starts_with("pid:")
    }

    fn normalize_window_id(&self, window_id: &str) -> Option<String> {
        if self.is_window_id(window_id) {
            Some(window_id.to_string())
        } else {
            None
        }
    }

    fn is_excluded_window_type(&self, _window_id: &str) -> Result<bool, String> {
        Ok(false)
    }

    fn is_hidden_window(&self, _window_id: &str) -> Result<bool, String> {
        // System Events cannot read miniaturized state of minimized windows on macOS.
        // Return true so try_restore_window proceeds to activate_window.
        Ok(true)
    }

    fn is_launcher_window(&self, window_id: &str) -> bool {
        let Some(pid) = parse_pid(window_id) else {
            return false;
        };
        process_name(pid as u32)
            .map(|name| {
                let lower = name.to_ascii_lowercase();
                LAUNCHER_MATCH_MARKERS.iter().any(|m| lower.contains(m))
            })
            .unwrap_or(false)
    }

    fn activate_window(&self, window_id: &str) -> Result<bool, String> {
        let pid = parse_pid(window_id).ok_or_else(|| format!("Invalid window ID: {window_id}"))?;
        Ok(ax::unminimize_and_raise(pid as i32))
    }

    fn window_pid(&self, window_id: &str) -> Result<Option<u32>, String> {
        Ok(parse_pid(window_id).map(|p| p as u32))
    }

    fn process_start_ticks(&self, pid: u32) -> Option<u64> {
        let output = Command::new("ps")
            .args(["-o", "lstart=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let raw = String::from_utf8_lossy(&output.stdout);
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        // Deterministic hash — DefaultHasher uses random seeding per process so
        // the hash from the minimize invocation would never match the restore invocation.
        Some(fnv1a(trimmed.as_bytes()))
    }
}

// -- Geometric actions --
//
// NSWorkspace.frontmostApplication is used instead of processes.whose({frontmost:true})
// because the latter races with process spawn timing and silently returns empty.
//
// NSRect from the ObjC bridge can be either:
//   flat:   {x, y, width, height}
//   nested: {origin: {x, y}, size: {width, height}}
// rf(r, flat, ns, ni) handles both.
//
// Coordinate conversion (Cocoa Y-up → AX Y-down):
//   ax.x = cocoa.x
//   ax.y = primaryScreenH - cocoa.y - frame.height

const SNAP_PREAMBLE: &str = r#"ObjC.import('AppKit');
const pid=$.NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;
const proc=Application('System Events').processes.whose({unixId:pid})[0];
const win=proc.windows[0];
const pos=win.position();const sz=win.size();
const cx=pos[0]+sz[0]/2;const cy=pos[1]+sz[1]/2;
function rf(r,f,ns,ni){return r[ns]!==undefined?Number(r[ns][ni]):Number(r[f]);}
const mf=$.NSScreen.mainScreen.frame;
const primaryH=rf(mf,'height','size','height');
function toAX(r){const ry=rf(r,'y','origin','y'),rh=rf(r,'height','size','height');return{x:rf(r,'x','origin','x'),y:primaryH-ry-rh,w:rf(r,'width','size','width'),h:rh};}
const all=$.NSScreen.screens;
let scr=toAX($.NSScreen.mainScreen.visibleFrame);
for(let i=0;i<all.count;i++){const ax=toAX(all.objectAtIndex(i).visibleFrame);if(cx>=ax.x&&cx<ax.x+ax.w&&cy>=ax.y&&cy<ax.y+ax.h){scr=ax;break;}}"#;

const MOVE_PREAMBLE: &str = r#"ObjC.import('AppKit');
const pid=$.NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;
const proc=Application('System Events').processes.whose({unixId:pid})[0];
const win=proc.windows[0];
const pos=win.position();const sz=win.size();
const cx=pos[0]+sz[0]/2;const cy=pos[1]+sz[1]/2;
function rf(r,f,ns,ni){return r[ns]!==undefined?Number(r[ns][ni]):Number(r[f]);}
const mf=$.NSScreen.mainScreen.frame;
const primaryH=rf(mf,'height','size','height');
function toAX(r){const ry=rf(r,'y','origin','y'),rh=rf(r,'height','size','height');return{x:rf(r,'x','origin','x'),y:primaryH-ry-rh,w:rf(r,'width','size','width'),h:rh};}
const all=$.NSScreen.screens;
const scrList=[];
for(let i=0;i<all.count;i++)scrList.push(toAX(all.objectAtIndex(i).visibleFrame));
scrList.sort((a,b)=>a.x-b.x);
let fromIdx=0;
for(let i=0;i<scrList.length;i++){if(cx>=scrList[i].x&&cx<scrList[i].x+scrList[i].w){fromIdx=i;break;}}"#;

fn run_snap(suffix: &str) -> Result<(), String> {
    run_jxa(&format!("{SNAP_PREAMBLE}\n{suffix}")).map(|_| ())
}

fn run_move(delta: i32) -> Result<(), String> {
    let suffix = format!(
        "const toIdx=(fromIdx+{}+scrList.length)%scrList.length;\
         const from=scrList[fromIdx];const to=scrList[toIdx];\
         const xR=(pos[0]-from.x)/from.w;const yR=(pos[1]-from.y)/from.h;\
         const wR=sz[0]/from.w;const hR=sz[1]/from.h;\
         win.position=[Math.round(to.x+xR*to.w),Math.round(to.y+yR*to.h)];\
         win.size=[Math.round(wR*to.w),Math.round(hR*to.h)];'ok'",
        delta
    );
    run_jxa(&format!("{MOVE_PREAMBLE}\n{suffix}")).map(|_| ())
}

pub fn snap_left() -> Result<(), String> {
    run_snap("win.position=[Math.round(scr.x),Math.round(scr.y)];win.size=[Math.round(scr.w/2),Math.round(scr.h)];'ok'")
}

pub fn snap_right() -> Result<(), String> {
    run_snap("win.position=[Math.round(scr.x+scr.w/2),Math.round(scr.y)];win.size=[Math.round(scr.w/2),Math.round(scr.h)];'ok'")
}

pub fn snap_bottom() -> Result<(), String> {
    run_snap("win.position=[Math.round(scr.x),Math.round(scr.y+scr.h/2)];win.size=[Math.round(scr.w),Math.round(scr.h/2)];'ok'")
}

pub fn maximize() -> Result<(), String> {
    run_snap("win.position=[Math.round(scr.x),Math.round(scr.y)];win.size=[Math.round(scr.w),Math.round(scr.h)];'ok'")
}

pub fn center() -> Result<(), String> {
    run_snap("const W=1152,H=892;win.position=[Math.round(scr.x+(scr.w-W)/2),Math.round(scr.y+(scr.h-H)/2)];win.size=[W,H];'ok'")
}

pub fn move_monitor_left() -> Result<(), String> {
    run_move(-1)
}

pub fn move_monitor_right() -> Result<(), String> {
    run_move(1)
}

// -- Accessibility API (raw FFI) --

mod ax {
    use std::ffi::{c_void, CString};

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> *mut c_void;
        fn AXUIElementCopyAttributeValue(
            element: *const c_void,
            attribute: *const c_void,
            value: *mut *mut c_void,
        ) -> i32;
        fn AXUIElementSetAttributeValue(
            element: *const c_void,
            attribute: *const c_void,
            value: *const c_void,
        ) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            c_str: *const i8,
            encoding: u32,
        ) -> *mut c_void;
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, idx: isize) -> *const c_void;
        fn CFRelease(cf: *const c_void);
        static kCFBooleanFalse: *const c_void;
        static kCFBooleanTrue: *const c_void;
    }

    const UTF8: u32 = 0x08000100;

    fn cfstr(s: &str) -> *mut c_void {
        let c = CString::new(s).unwrap();
        unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) }
    }

    pub fn unminimize_and_raise(pid: i32) -> bool {
        unsafe {
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return false;
            }

            let ax_windows = cfstr("AXWindows");
            let mut windows_ref: *mut c_void = std::ptr::null_mut();
            let err = AXUIElementCopyAttributeValue(app, ax_windows, &mut windows_ref);
            CFRelease(ax_windows);

            if err != 0 || windows_ref.is_null() {
                CFRelease(app);
                return false;
            }

            // Only unminimize the first minimized window, not all of them.
            let ax_minimized = cfstr("AXMinimized");
            let count = CFArrayGetCount(windows_ref);
            for i in 0..count {
                let win = CFArrayGetValueAtIndex(windows_ref, i);
                let mut val: *mut c_void = std::ptr::null_mut();
                let err = AXUIElementCopyAttributeValue(win, ax_minimized, &mut val);
                if err == 0 && !val.is_null() && val == kCFBooleanTrue as *mut c_void {
                    AXUIElementSetAttributeValue(win, ax_minimized, kCFBooleanFalse);
                    break;
                }
            }
            CFRelease(ax_minimized);

            // Bring app to front
            let ax_frontmost = cfstr("AXFrontmost");
            AXUIElementSetAttributeValue(app, ax_frontmost, kCFBooleanTrue);
            CFRelease(ax_frontmost);

            CFRelease(windows_ref);
            CFRelease(app);
            true
        }
    }
}

// -- Helpers --

fn run_jxa(script: &str) -> Result<String, String> {
    let output = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", script])
        .output()
        .map_err(|e| format!("Failed to run osascript: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!(
            "osascript failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn parse_pid(window_id: &str) -> Option<i64> {
    window_id.split(':').nth(1)?.parse().ok()
}

fn parse_idx(window_id: &str) -> Option<usize> {
    window_id.split(':').nth(2)?.parse().ok()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn process_name(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}
