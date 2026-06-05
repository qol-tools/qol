#!/usr/bin/env python3
import time
import os
import sys
import re

LOG_FILE = "/tmp/qol-altmon.log"

# ANSI Terminal Colors
COLOR_RESET = "\033[0m"
COLOR_HEADER = "\033[1;36m"
COLOR_TIME = "\033[2m"
COLOR_PICK = "\033[1;34m"
COLOR_FOCUS = "\033[1;33m"
COLOR_AMC = "\033[1;35m"
COLOR_OPACITY = "\033[1;36m"
COLOR_OK = "\033[1;32m"
COLOR_WARN = "\033[1;33m"
COLOR_FAIL = "\033[1;31m"
COLOR_DIM = "\033[2m"
COLOR_HOTKEY = "\033[1;35m"

# State tracking to filter spam
last_winner = None
last_cursor_pos = None
last_focus_pos = None

# Buffering for group rendering
event_buffer = []
last_event_real_time = 0
ghost_dump_active = False
dumped_windows = []

# Expected/target opacities tracking to detect discrepancies
target_opacities = {}

# Dynamic monitor bounds registry: list of (x, y, w, h)
monitors = []

def register_monitor(x, y, w, h):
    global monitors
    bounds = (int(float(x)), int(float(y)), int(float(w)), int(float(h)))
    if bounds not in monitors:
        monitors.append(bounds)
        # Sort monitors left-to-right (x), then top-to-bottom (y)
        monitors.sort(key=lambda m: (m[0], m[1]))

def get_monitor_name(x, y):
    try:
        x, y = int(float(x)), int(float(y))
    except (ValueError, TypeError):
        return f"({x},{y})"
    for idx, (mx, my, mw, mh) in enumerate(monitors):
        if mx <= x < mx + mw and my <= y < my + mh:
            return f"Mon {idx}"
    return f"({x},{y})"

def get_monitor_name_by_origin(ox, oy):
    try:
        ox, oy = int(float(ox)), int(float(oy))
    except (ValueError, TypeError):
        return f"({ox},{oy})"
    for idx, (mx, my, mw, mh) in enumerate(monitors):
        if mx == ox and my == oy:
            return f"Mon {idx}"
    return f"({ox},{oy})"

def format_title_compact(title):
    # Parse title like qol-alt-tab-picker@1920,0,2560x1440 to qol-alt-tab-picker@Mon 1
    m = re.match(r"(?P<prefix>qol-[^@]+)@(?P<x>-?\d+),(?P<y>-?\d+)(?:,\d+x\d+)?", title)
    if m:
        mon = get_monitor_name_by_origin(m.group("x"), m.group("y"))
        return f"{m.group('prefix')}@{mon}"
    return title

def format_timestamp(unix_ms_str):
    try:
        t_sec = float(unix_ms_str) / 1000.0
        return time.strftime("%H:%M:%S", time.localtime(t_sec)) + f".{int(unix_ms_str)[-3:]}"
    except Exception:
        return unix_ms_str

def get_process_name(pid):
    try:
        with open(f"/proc/{pid}/comm", "r") as f:
            return f.read().strip()
    except Exception:
        return str(pid)

def parse_line(line):
    # Format: {unix_ms} pid={pid} {TAG} {msg}
    m = re.match(r"(?P<ts>\d+)\s+pid=(?P<pid>\d+)\s+(?P<tag>\w+)\s+(?P<msg>.*)", line)
    if m:
        return m.group("ts"), m.group("pid"), m.group("tag"), m.group("msg")
    return None, None, None, None

def process_line(ts_raw, pid, tag, msg):
    global last_winner, last_cursor_pos, last_focus_pos
    global event_buffer, last_event_real_time, ghost_dump_active, dumped_windows
    global target_opacities
    
    ts = format_timestamp(ts_raw)
    
    # Auto-register any monitor geometries found in logs
    for m in re.finditer(r"@(?P<x>-?\d+),(?P<y>-?\d+),(?P<w>\d+)x(?P<h>\d+)", msg):
        register_monitor(m.group("x"), m.group("y"), m.group("w"), m.group("h"))
    for m in re.finditer(r"MonitorBounds \{\s*x:\s*(?P<x>[\d.]+),\s*y:\s*(?P<y>[\d.]+),\s*width:\s*(?P<w>[\d.]+),\s*height:\s*(?P<h>[\d.]+)\s*\}", msg):
        register_monitor(m.group("x"), m.group("y"), m.group("w"), m.group("h"))
    
    if tag == "PICK":
        m = re.search(r"cursor=\((?P<cx>-?\d+),(?P<cy>-?\d+)\)\s+cursor_age_ms=(?P<cage>\d+)\s+focus=\((?P<fx>-?\d+),(?P<fy>-?\d+)\)\s+focus_age_ms=(?P<fage>\d+)\s+winner=(?P<winner>\w+)", msg)
        if m:
            winner = m.group("winner")
            cx, cy = m.group("cx"), m.group("cy")
            fx, fy = m.group("fx"), m.group("fy")
            cage = int(m.group("cage"))
            fage = int(m.group("fage"))
            
            if winner != last_winner or (cx, cy) != last_cursor_pos or (fx, fy) != last_focus_pos:
                last_winner = winner
                last_cursor_pos = (cx, cy)
                last_focus_pos = (fx, fy)
                
                cursor_mon = get_monitor_name(cx, cy)
                focus_mon = get_monitor_name(fx, fy)
                
                cursor_status = f"{COLOR_FAIL}(STALE){COLOR_RESET}" if cage >= 1500 else f"{COLOR_OK}(ACTIVE){COLOR_RESET}"
                focus_status = f"{COLOR_FAIL}(STALE){COLOR_RESET}" if fage >= 1500 else f"{COLOR_OK}(ACTIVE){COLOR_RESET}"
                
                winner_color = COLOR_OK if winner == "cursor" else COLOR_FOCUS
                text = (f"{COLOR_PICK}[PICK]{COLOR_RESET} Winner -> {winner_color}{winner.upper()}{COLOR_RESET} | "
                        f"Cursor: {cursor_mon} (age: {cage/1000.0:.2f}s {cursor_status}) | "
                        f"Focus: {focus_mon} (age: {fage/1000.0:.2f}s {focus_status})")
                event_buffer.append((int(ts_raw), ts, "PICK", text))
                last_event_real_time = time.time()
                      
    elif tag == "FOCUS_WIN":
        m = re.search(r'winpos=\((?P<x>-?\d+),(?P<y>-?\d+)[^)]*\)\s+title="(?P<title>[^"]+)"', msg)
        if m:
            title = m.group("title")
            x, y = m.group("x"), m.group("y")
            short_title = title[:30] + "..." if len(title) > 30 else title
            mon = get_monitor_name(x, y)
            
            pid_m = re.search(r"pid=(?P<pid>\d+)", msg)
            proc_info = ""
            if pid_m:
                pid = pid_m.group("pid")
                try:
                    with open(f"/proc/{pid}/comm", "r") as f:
                        proc_name = f.read().strip()
                    proc_info = f" (proc: {proc_name}, pid: {pid})"
                except Exception:
                    proc_info = f" (pid: {pid})"
            
            text = f"{COLOR_FOCUS}[FOCUS]{COLOR_RESET} Active window: \"{short_title}\"{proc_info} on {mon}"
            event_buffer.append((int(ts_raw), ts, "FOCUS", text))
            last_event_real_time = time.time()
                  
    elif tag == "AMC":
        m = re.search(r"active_visible=(?P<title>\S+)", msg)
        if m:
            title = m.group("title")
            comp_title = format_title_compact(title)
            text = f"{COLOR_AMC}[ACTIVE_MON_CHANGED]{COLOR_RESET} Target -> {COLOR_AMC}{comp_title}{COLOR_RESET}"
            event_buffer.append((int(ts_raw), ts, "AMC", text))
            last_event_real_time = time.time()
                  
    elif tag == "HIDE_WIN":
        m = re.search(r"title=(?P<title>\S+)\s+wid=\d+\s+path=\S+\s+opacity=(?P<opacity>[\d.]+)", msg)
        if m:
            title = m.group("title")
            opacity = float(m.group("opacity"))
            op_color = COLOR_OK if opacity > 0.0 else COLOR_DIM
            comp_title = format_title_compact(title)
            target_opacities[comp_title] = opacity
            proc_name = get_process_name(pid)
            text = f"{COLOR_OPACITY}[OPACITY]{COLOR_RESET} {comp_title} -> {op_color}{opacity}{COLOR_RESET} (by {proc_name})"
            event_buffer.append((int(ts_raw), ts, "HIDE_WIN", text))
            last_event_real_time = time.time()

    elif tag == "SHOW_WIN":
        m = re.search(r"title=(?P<title>\S+)\s+wid=\d+\s+cleared_opacity->(?P<opacity>\d+(?:\.\d+)?)", msg)
        if m:
            title = m.group("title")
            opacity = float(m.group("opacity"))
            comp_title = format_title_compact(title)
            target_opacities[comp_title] = opacity
            proc_name = get_process_name(pid)
            text = f"{COLOR_OPACITY}[OPACITY]{COLOR_RESET} {comp_title} -> {COLOR_OK}{opacity}{COLOR_RESET} (by {proc_name})"
            event_buffer.append((int(ts_raw), ts, "SHOW_WIN", text))
            last_event_real_time = time.time()

    elif tag == "ALT_POLL_START":
        m = re.search(r"title=(?P<title>\S+)", msg)
        title = m.group("title") if m else "alt-tab"
        comp_title = format_title_compact(title)
        
        target_opacities[comp_title] = 1.0
        prefix = comp_title.split("@")[0]
        for k in list(target_opacities.keys()):
            if k.startswith(prefix) and k != comp_title:
                target_opacities[k] = 0.0
                
        proc_name = get_process_name(pid)
        text = f"{COLOR_HOTKEY}[TRIGGER]{COLOR_RESET} Alt-Tab Picker Opened ({comp_title}) (by {proc_name})"
        event_buffer.append((int(ts_raw), ts, "TRIGGER", text))
        last_event_real_time = time.time()

    elif tag == "DISMISS":
        m = re.search(r"from=(?P<src>\S+)", msg)
        src = m.group("src") if m else "unknown"
        src_color = COLOR_OK if ("alt-up" in src or "super-up" in src or "modifiers/" in src) else COLOR_WARN
        proc_name = get_process_name(pid)
        text = f"{COLOR_HOTKEY}[DISMISS]{COLOR_RESET} Picker closed (from={src_color}{src}{COLOR_RESET}) (by {proc_name})"
        event_buffer.append((int(ts_raw), ts, "DISMISS", text))
        last_event_real_time = time.time()
                  
    elif tag == "GHOSTDUMP":
        if "begin" in msg:
            ghost_dump_active = True
            dumped_windows = []
        elif "end" in msg:
            ghost_dump_active = False
            active_ghosts = []
            active_pickers = []
            inactive_visible = []
            
            seen_titles = set()
            plugin_wins = {}
            divergence_msgs = []
            for w_info in dumped_windows:
                title, opacity, role, map_state, owner_pid, actual_x, actual_y, actual_w, actual_h = w_info
                if title in seen_titles:
                    continue
                seen_titles.add(title)
                
                comp_title = format_title_compact(title)
                map_suffix = "" if map_state == "viewable" else f" ({map_state})"
                
                proc_name = "unknown"
                if owner_pid:
                    try:
                        with open(f"/proc/{owner_pid}/comm", "r") as f:
                            proc_name = f.read().strip()
                    except Exception:
                        pass
                if proc_name == "unknown":
                    if "alt-tab" in title:
                        proc_name = "alt-tab"
                    elif "launcher" in title:
                        proc_name = "launcher"
                
                proc_suffix = f"/{proc_name}" if proc_name != "unknown" else ""
                
                if opacity > 0.0:
                    plugin_wins.setdefault(proc_name, []).append((comp_title, opacity, map_suffix))
                    
                    if role == "ghost":
                        active_ghosts.append(f"{comp_title}{proc_suffix}({opacity}{map_suffix})")
                    elif role == "live" and ("alt-tab" in title or "launcher" in title):
                        active_pickers.append(f"{comp_title}{proc_suffix}({opacity}{map_suffix})")
                    elif role == "invisible":
                        inactive_visible.append(f"{comp_title}{proc_suffix}({opacity}{map_suffix})")
                
                # Verify opacity against target
                expected_opacity = target_opacities.get(comp_title, 0.0)
                is_actually_hidden = (map_state != "viewable" or opacity <= 0.01)
                is_expected_hidden = (expected_opacity <= 0.01)
                
                if not (is_actually_hidden and is_expected_hidden):
                    # We have a divergence if they don't match
                    if abs(opacity - expected_opacity) > 0.01:
                        divergence_msgs.append(
                            f"{comp_title} opacity is {opacity}{map_suffix}, expected {expected_opacity}"
                        )
                
                # Verify position (monitor alignment) if active/visible
                if not is_actually_hidden:
                    title_m = re.match(r"qol-[^@]+@(?P<tx>-?\d+),(?P<ty>-?\d+)", title)
                    if title_m:
                        tx = int(title_m.group("tx"))
                        ty = int(title_m.group("ty"))
                        expected_mon = get_monitor_name_by_origin(tx, ty)
                        actual_mon = get_monitor_name(actual_x, actual_y)
                        if actual_mon != expected_mon:
                            divergence_msgs.append(
                                f"{comp_title} is on {actual_mon} (expected {expected_mon})"
                            )
            
            # Check invariant: at most one active/visible window per plugin
            for proc, wins in plugin_wins.items():
                if proc != "unknown" and len(wins) > 1:
                    comp_wins = [f"{t}({op}{ms})" for t, op, ms in wins]
                    inactive_visible.append(f"Multiple active {proc}: {', '.join(comp_wins)}")
                        
            status = COLOR_OK + "OK" + COLOR_RESET
            all_divergences = inactive_visible + divergence_msgs
            if all_divergences:
                status = COLOR_FAIL + f"DIVERGENCE: {', '.join(all_divergences)}" + COLOR_RESET
                
            active_parts = []
            if active_ghosts:
                active_parts.append(f"Active Ghost: {', '.join(active_ghosts)}")
            if active_pickers:
                active_parts.append(f"Active Picker: {', '.join(active_pickers)}")
                
            text = f"{COLOR_DIM}[STATE]{COLOR_RESET} {' | '.join(active_parts) or 'No Active Win'} | {status}"
            
            if event_buffer and event_buffer[-1][2] == "SUMMARY" and event_buffer[-1][3] == text:
                pass
            else:
                event_buffer.append((int(ts_raw), ts, "SUMMARY", text))
            last_event_real_time = time.time()
                  
    elif tag == "GHOSTWIN" and ghost_dump_active:
        m = re.search(r"title=(?P<title>\S+)\s+owner_pid=(?P<owner_pid>\d+)\s+wid=\d+\s+pos=\((?P<x>-?\d+),(?P<y>-?\d+)\)\s+size=(?P<w>\d+)x(?P<h>\d+)\s+opacity=(?P<opacity>\S+)\s+map=(?P<map>\S+)\s+role=(?P<role>\S+)", msg)
        if m:
            title = m.group("title")
            role = m.group("role")
            map_state = m.group("map")
            opacity_str = m.group("opacity")
            owner_pid = m.group("owner_pid")
            opacity = 1.0 if opacity_str == "unset" else float(opacity_str)
            x = int(m.group("x"))
            y = int(m.group("y"))
            w = int(m.group("w"))
            h = int(m.group("h"))
            dumped_windows.append((title, opacity, role, map_state, owner_pid, x, y, w, h))

def flush_buffer():
    global event_buffer
    if not event_buffer:
        return
        
    t_root_ms, ts_root, _, text_root = event_buffer[0]
    
    t_last_ms = event_buffer[-1][0]
    latency_ms = t_last_ms - t_root_ms
    latency_color = COLOR_FAIL if latency_ms > 100 else (COLOR_WARN if latency_ms > 50 else COLOR_OK)
    latency_str = f" {latency_color}(latency: {latency_ms}ms){COLOR_RESET}" if latency_ms > 0 else ""
    
    print(f"{COLOR_TIME}[{ts_root}]{COLOR_RESET} ┌── {text_root}{latency_str}")
    
    n = len(event_buffer)
    for idx in range(1, n):
        _, ts, _, text = event_buffer[idx]
        connector = "└── " if idx == n - 1 else "├── "
        print(f"{COLOR_TIME}[{ts}]{COLOR_RESET} │   {connector}{text}")
        
    print()
    event_buffer = []

def query_initial_monitors():
    try:
        import subprocess
        out = subprocess.check_output(["xrandr", "--current"], stderr=subprocess.DEVNULL).decode()
        for line in out.splitlines():
            if " connected" in line:
                m = re.search(r"\b(?P<w>\d+)x(?P<h>\d+)\+(?P<x>-?\d+)\+(?P<y>-?\d+)", line)
                if m:
                    register_monitor(m.group("x"), m.group("y"), m.group("w"), m.group("h"))
    except Exception:
        pass

def main():
    global last_event_real_time
    if not os.path.exists(LOG_FILE):
        print(f"Error: Log file {LOG_FILE} does not exist yet. Please run the daemon.")
        sys.exit(1)
        
    print(f"{COLOR_HEADER}Tailing /tmp/qol-altmon.log with dynamic monitor layout mapping...{COLOR_RESET}")
    print(f"{COLOR_DIM}Aggregating transitions into beautiful call tree structures with latencies.{COLOR_RESET}\n")
    
    query_initial_monitors()
    
    f = open(LOG_FILE, "r")
    f.seek(0, os.SEEK_END)
    
    try:
        while True:
            if event_buffer and (time.time() - last_event_real_time > 0.08):
                flush_buffer()
                
            line = f.readline()
            if not line:
                time.sleep(0.01)
                continue
            ts, pid, tag, msg = parse_line(line.strip())
            if ts and pid and tag and msg:
                process_line(ts, pid, tag, msg)
    except KeyboardInterrupt:
        flush_buffer()
        print(f"\n{COLOR_HEADER}Exiting tailer.{COLOR_RESET}")
        sys.exit(0)

if __name__ == "__main__":
    main()
