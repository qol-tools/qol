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
COLOR_SUCCESS = COLOR_OK
COLOR_WARN = "\033[1;33m"
COLOR_FAIL = "\033[1;31m"
COLOR_DIM = "\033[2m"
COLOR_HOTKEY = "\033[1;35m"

# State tracking to filter spam
last_winner = None
last_cursor_pos = None
last_focus_pos = None
last_printed_opacities = {}
last_printed_statuses = {}
last_printed_state_summary = None

# Focus/Activation tracking state
pending_activation = None
last_parsed_ts = 0
winact_fail_pids = set()

# Buffering for group rendering
event_buffer = []
last_event_real_time = 0
ghost_dump_active = False
dumped_windows = []

# Expected/target opacities tracking to detect discrepancies
target_opacities = {}
picker_status = {}

# Opacity write churn instrumentation: every HIDE_WIN/SHOW_WIN is a popup
# visibility write. Cached window ids avoid repeat _NET_CLIENT_LIST scans on hot paths.
REVERT_WINDOW_MS = 200
opacity_state = {}
waste = {
    "writes": 0,
    "redundant": 0,
    "reverts": 0,
    "by_reason": {},
    "redundant_by_reason": {},
    "revert_pairs": {},
}

# Dynamic monitor bounds registry: list of (x, y, w, h)
monitors = []

# Aggregate stats for --stats / --replay summaries
stats = {
    "focus_req": 0,
    "focus_ok": 0,
    "focus_misdirect": 0,
    "focus_timeout": 0,
    "supersede": 0,
    "divergence": 0,
    "oscillation": 0,
    "latencies": [],
    "focus_history": [],
}
last_divergence = None

ANOMALY_MARKERS = ("MISDIRECTED", "FOCUS FAILURE", "SUPERSEDED", "DIVERGENCE", "Timed out", "THRASH", "REVERT")

import argparse

# Pre-process arguments for legacy positional command style
sys_args = sys.argv[1:]
legacy_focus = False
legacy_plugin = None

positionals = [arg for arg in sys_args if not arg.startswith("-")]
if "focus" in positionals:
    legacy_focus = True
    sys_args = [arg for arg in sys_args if arg != "focus"]

positionals = [arg for arg in sys_args if not arg.startswith("-")]
if positionals:
    legacy_plugin = positionals[0]
    sys_args = [arg for arg in sys_args if arg != legacy_plugin]

parser = argparse.ArgumentParser(description="QoL Compact Tracer")
parser.add_argument("plugin", nargs="?", default=legacy_plugin, help="Plugin name to filter by")
parser.add_argument("-f", "--focus-only", action="store_true", default=legacy_focus, help="Focus events only")
parser.add_argument("-g", "--no-ghosts", action="store_true", help="Hide ghost window dumps")
parser.add_argument("-o", "--no-opacity", action="store_true", help="Hide opacity events")
parser.add_argument("--topic", choices=["focus", "monitor", "boot", "opacity", "ui", "preview", "all"], default="all", help="Slice trace by topic")
parser.add_argument("--grep", help="Filter output lines by substring")
parser.add_argument("--since", help="Filter events since duration (e.g. 5s, 10m)")
parser.add_argument("--mark", help="Inject a custom marker label into the log and exit")
parser.add_argument("--stats", action="store_true", help="Accumulate focus/latency stats and print a summary on exit")
parser.add_argument("--replay", action="store_true", help="Process the whole existing log from the start, then exit (pairs with --stats)")
parser.add_argument("--anomalies", action="store_true", help="Show only anomalies: misdirects, timeouts, supersedes, divergences")

args = parser.parse_args(sys_args)

if args.focus_only:
    args.topic = "focus"

if args.plugin == "runtime":
    args.plugin = None
filter_plugin = args.plugin

def register_monitor(x, y, w, h):
    global monitors
    bounds = (int(float(x)), int(float(y)), int(float(w)), int(float(h)))
    if bounds not in monitors:
        monitors.append(bounds)
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
    m = re.match(r"(?P<prefix>qol-[^@]+)@(?P<x>-?\d+),(?P<y>-?\d+)(?:,\d+x\d+)?", title)
    if m:
        mon = get_monitor_name_by_origin(m.group("x"), m.group("y"))
        return f"{m.group('prefix')}@{mon}"
    return title

def format_timestamp(unix_ms_str):
    try:
        t_sec = float(unix_ms_str) / 1000.0
        return time.strftime("%H:%M:%S", time.localtime(t_sec)) + f".{unix_ms_str[-3:]}"
    except Exception:
        return unix_ms_str

def hash_color(name):
    colors = [
        "\033[1;34m",  # Bold Blue
        "\033[1;35m",  # Bold Magenta
        "\033[1;36m",  # Bold Cyan
        "\033[1;32m",  # Bold Green
        "\033[1;94m",  # Bold Light Blue
        "\033[1;95m",  # Bold Light Magenta
        "\033[1;96m",  # Bold Light Cyan
        "\033[1;92m",  # Bold Light Green
    ]
    if name in ("host", "qol-tray", "tray"):
        return "\033[1;33m"  # Yellow/Gold
    h = 0
    for char in name:
        h = ord(char) + ((h << 5) - h)
    idx = abs(h) % len(colors)
    return colors[idx]

# pid -> process name, cached so a long replay never spawns ps per line.
_proc_name_cache = {}

def get_process_name(pid):
    cached = _proc_name_cache.get(pid)
    if cached is not None:
        return cached
    name = str(pid)
    try:
        with open(f"/proc/{pid}/comm", "r") as f:
            name = f.read().strip()
    except Exception:
        # macOS has no /proc; fall back to ps short command name (ucomm).
        try:
            import subprocess
            out = subprocess.check_output(
                ["ps", "-p", str(pid), "-o", "ucomm="],
                stderr=subprocess.DEVNULL,
            ).decode().strip()
            if out:
                name = out
        except Exception:
            pass
    _proc_name_cache[pid] = name
    return name

def reason_suffix(msg):
    m = re.search(r"\breason=(?P<reason>\S+)", msg)
    if not m or m.group("reason") == "?":
        return ""
    return f" {COLOR_DIM}(why: {m.group('reason')}){COLOR_RESET}"

def extract_reason(msg):
    m = re.search(r"\breason=(?P<reason>\S+)", msg)
    return m.group("reason") if m else "?"

def set_opacity_state(comp_title, opacity, reason, ts_raw):
    st = opacity_state.get(comp_title)
    if st is None:
        opacity_state[comp_title] = {"op": opacity, "reason": reason, "ts": ts_raw, "prev_op": None}
    elif abs(st["op"] - opacity) >= 0.001:
        opacity_state[comp_title] = {"op": opacity, "reason": reason, "ts": ts_raw, "prev_op": st["op"]}
    else:
        st["reason"] = reason
        st["ts"] = ts_raw


def record_opacity_write(comp_title, opacity, reason, ts_raw):
    waste["writes"] += 1
    waste["by_reason"][reason] = waste["by_reason"].get(reason, 0) + 1
    st = opacity_state.get(comp_title)
    classification = None
    if st is not None:
        if abs(st["op"] - opacity) < 0.001:
            waste["redundant"] += 1
            waste["redundant_by_reason"][reason] = waste["redundant_by_reason"].get(reason, 0) + 1
            classification = ("redundant", st["reason"], ts_raw - st["ts"])
        elif (st["prev_op"] is not None and abs(st["prev_op"] - opacity) < 0.001
              and st["reason"] != reason and ts_raw - st["ts"] <= REVERT_WINDOW_MS):
            waste["reverts"] += 1
            pair = f"{st['reason']}->{reason}"
            waste["revert_pairs"][pair] = waste["revert_pairs"].get(pair, 0) + 1
            classification = ("revert", st["reason"], ts_raw - st["ts"])
    set_opacity_state(comp_title, opacity, reason, ts_raw)
    return classification

def churn_suffix(cls):
    if cls and cls[0] == "revert":
        return f" {COLOR_FAIL}⟲ REVERT {cls[1]}@{cls[2]}ms{COLOR_RESET}"
    return ""

def write_attribution(comp_title, now_ts):
    st = opacity_state.get(comp_title)
    if not st:
        return ""
    return f" ←{st['reason']} {now_ts - st['ts']}ms ago"

def print_waste():
    total = waste["writes"]
    if total == 0:
        return
    print(f"\n{COLOR_HEADER}═══ OPACITY CHURN ═══{COLOR_RESET}")
    print(f"  Opacity writes:      {total}  {COLOR_DIM}(each = popup visibility write; cached WID avoids repeat scans){COLOR_RESET}")
    print(f"  {COLOR_WARN}Redundant (no-op){COLOR_RESET}:   {waste['redundant']} ({100 * waste['redundant'] // total}%)  {COLOR_DIM}burned round-trips{COLOR_RESET}")
    print(f"  {COLOR_FAIL}Reverts (self-heal){COLOR_RESET}: {waste['reverts']}")
    if waste["by_reason"]:
        print("  Writes by reason:")
        for reason, c in sorted(waste["by_reason"].items(), key=lambda kv: -kv[1]):
            red = waste["redundant_by_reason"].get(reason, 0)
            redstr = f"  {COLOR_DIM}({red} redundant){COLOR_RESET}" if red else ""
            print(f"    {reason:<10} {c}{redstr}")
    if waste["revert_pairs"]:
        print(f"  {COLOR_FAIL}Self-heal pairs{COLOR_RESET} {COLOR_DIM}(firepit -> firefighter){COLOR_RESET}:")
        for pair, c in sorted(waste["revert_pairs"].items(), key=lambda kv: -kv[1]):
            print(f"    {pair:<22} ×{c}")
    print(f"{COLOR_HEADER}═════════════════════{COLOR_RESET}")

def percentile(values, p):
    if not values:
        return 0
    ordered = sorted(values)
    k = int(round((p / 100.0) * (len(ordered) - 1)))
    return ordered[max(0, min(len(ordered) - 1, k))]

def record_focus_ok(ts_ms, wid, latency):
    stats["focus_ok"] += 1
    stats["latencies"].append(latency)
    history = stats["focus_history"]
    history.append((ts_ms, wid))
    while history and ts_ms - history[0][0] > 2000:
        history.pop(0)
    if len(history) >= 3 and history[-1][1] == history[-3][1] != history[-2][1]:
        stats["oscillation"] += 1

def print_stats():
    if not args.stats:
        return
    resolved = stats["focus_ok"] + stats["focus_misdirect"] + stats["focus_timeout"]
    lat = stats["latencies"]
    print(f"\n{COLOR_HEADER}═══ SESSION STATS ═══{COLOR_RESET}")
    print(f"  Focus requests sent:  {stats['focus_req']}")
    print(f"  Focus resolved:       {resolved}")
    print(f"    {COLOR_OK}✔ success{COLOR_RESET}      {stats['focus_ok']}")
    print(f"    {COLOR_WARN}⚠ misdirected{COLOR_RESET}  {stats['focus_misdirect']}")
    print(f"    {COLOR_FAIL}✖ timed out{COLOR_RESET}    {stats['focus_timeout']}")
    print(f"    ⚡ superseded   {stats['supersede']}")
    print(f"    ⟳ oscillations {stats['oscillation']}")
    print(f"    ⚠ divergences  {stats['divergence']}")
    if lat:
        print(f"  Focus latency ms: p50={percentile(lat, 50)} p95={percentile(lat, 95)} "
              f"max={max(lat)} min={min(lat)} (n={len(lat)})")
    print(f"{COLOR_HEADER}═════════════════════{COLOR_RESET}")

def parse_line(line):
    m = re.match(r"(?P<ts>\d+)\s+pid=(?P<pid>\d+)\s+(?P<tag>\w+)\s+(?P<msg>.*)", line)
    if m:
        return m.group("ts"), m.group("pid"), m.group("tag"), m.group("msg")
    return None, None, None, None

def match_topic(tag, topic):
    if not topic or topic == "all":
        return True
    if topic == "ui":
        return tag.startswith("LAUNCHER_") or tag.startswith("WORLD_")
    if topic == "preview":
        return (
            tag.startswith("PREVIEW_")
            or tag.startswith("REFRESH_")
            or tag.startswith("CAPTURE")
            or tag in ("SHOW_RECV", "SHOW_TIMING", "SHOW_PAINTED", "FOCUS_WIN")
        )
    categories = {
        "focus": ("FOCUS", "FOCUS_WIN", "ACTIVATE", "ACTIVATE_WIN", "WM_RECEIVE", "ALT_POLL_START", "DISMISS"),
        "monitor": ("PUBLISH", "SUBSCRIBE", "RECV", "LEGEND", "AMC", "HOST_EMIT_AMC", "PLUGIN_RECV_AMC"),
        "boot": ("PUBLISH", "SUBSCRIBE", "RECV", "LEGEND"),
        "opacity": ("SHOW_WIN", "HIDE_WIN", "GHOSTWIN", "GHOSTDUMP", "SUMMARY")
    }
    return tag in categories.get(topic, ())

def launcher_field(msg, name):
    m = re.search(rf"\b{name}=(?P<value>\S+)", msg)
    return m.group("value") if m else "?"

def launcher_quoted(msg, name):
    m = re.search(rf'\b{name}="(?P<value>[^"]*)"', msg)
    return m.group("value") if m else ""

def launcher_window(msg):
    m = re.search(r"\bwin=\((?P<x>-?\d+),(?P<y>-?\d+),(?P<w>\d+)x(?P<h>\d+)\)", msg)
    if not m:
        return "win=?"
    return f"{m.group('w')}x{m.group('h')}@({m.group('x')},{m.group('y')})"

def format_launcher_event(tag, msg):
    if tag == "LAUNCHER_SHOW":
        title = format_title_compact(launcher_field(msg, "title"))
        path = launcher_field(msg, "path")
        m = re.search(r"\bpos=\((?P<x>-?\d+),(?P<y>-?\d+)\)\s+size=(?P<w>\d+)x(?P<h>\d+)", msg)
        if m:
            return (f"Launcher show {COLOR_OK}{path}{COLOR_RESET} {title} "
                    f"{m.group('w')}x{m.group('h')}@({m.group('x')},{m.group('y')})")
        return f"Launcher show {COLOR_OK}{path}{COLOR_RESET} {title}"

    if tag == "LAUNCHER_INPUT":
        effect = launcher_field(msg, "effect")
        key = launcher_field(msg, "key")
        q = launcher_quoted(msg, "q")
        selected = launcher_field(msg, "selected")
        results = launcher_field(msg, "results_before")
        return (f"Launcher input {COLOR_HOTKEY}{key}{COLOR_RESET} -> {effect} "
                f"q=\"{q}\" selected={selected} results_before={results}")

    if tag == "LAUNCHER_RESIZE":
        q = launcher_quoted(msg, "q")
        rows = launcher_field(msg, "rows")
        results = launcher_field(msg, "results")
        from_h = launcher_field(msg, "from_h")
        to_h = launcher_field(msg, "to_h")
        return (f"{COLOR_OPACITY}Launcher resize{COLOR_RESET} h {from_h}->{to_h} "
                f"rows={rows} results={results} q=\"{q}\" {launcher_window(msg)}")

    if tag == "LAUNCHER_RENDER":
        q = launcher_quoted(msg, "q")
        selected_name = launcher_quoted(msg, "selected_name")
        results = launcher_field(msg, "results")
        visible = launcher_field(msg, "visible")
        selected = launcher_field(msg, "selected")
        scroll = launcher_field(msg, "scroll")
        hidden = launcher_field(msg, "hidden")
        target_h = launcher_field(msg, "target_h")
        visual_h = launcher_field(msg, "visual_h")
        total_us = launcher_field(msg, "total_us")
        filter_us = launcher_field(msg, "filter_us")
        rows_us = launcher_field(msg, "rows_us")
        return (f"Launcher render q=\"{q}\" results={results} visible={visible} "
                f"selected={selected} \"{selected_name}\" scroll={scroll} hidden={hidden} "
                f"{launcher_window(msg)} target_h={target_h} visual_h={visual_h} "
                f"{COLOR_DIM}time={total_us}us filter={filter_us}us rows={rows_us}us{COLOR_RESET}")

    if tag == "LAUNCHER_DISMISS":
        src = launcher_field(msg, "from")
        q = launcher_quoted(msg, "q")
        results = launcher_field(msg, "results")
        selected = launcher_field(msg, "selected")
        selected_name = launcher_quoted(msg, "selected_name")
        return (f"Launcher closed from={COLOR_WARN}{src}{COLOR_RESET} q=\"{q}\" "
                f"results={results} selected={selected} \"{selected_name}\"")

    return f"{tag}: {msg}"

def winact_ms_color(value):
    return COLOR_FAIL if value > 100 else (COLOR_WARN if value > 50 else COLOR_OK)

def winact_outcome_color(outcome):
    if outcome == "ok":
        return COLOR_OK
    if outcome == "fail":
        return COLOR_FAIL
    return COLOR_DIM

def winact_int(value):
    return int(value) if value.isdigit() else None

def format_winact_event(tag, msg, partial=False):
    if tag == "WINACT_AX":
        op = launcher_field(msg, "op")
        pid = launcher_field(msg, "pid")
        dur = launcher_field(msg, "dur_ms")
        outcome = launcher_field(msg, "outcome")
        dur_ms = winact_int(dur)
        dur_str = f"{dur}ms" if dur_ms is not None else "?ms"
        pid_suffix = f" pid={pid}" if pid not in ("0", "-1", "?") else ""
        return (f"  {COLOR_DIM}AX{COLOR_RESET} {op}{pid_suffix} "
                f"{winact_ms_color(dur_ms or 0)}{dur_str}{COLOR_RESET} "
                f"{winact_outcome_color(outcome)}{outcome}{COLOR_RESET}")

    if tag == "WINACT_MINIMIZE":
        branch = launcher_field(msg, "branch")
        visible = launcher_field(msg, "visible")
        regular = launcher_field(msg, "regular")
        outcome = launcher_field(msg, "outcome")
        label = "hide (instant)" if branch == "hide" else "minimize (animated)"
        return (f"  {COLOR_DIM}strategy{COLOR_RESET} {label} "
                f"visible={visible} regular={regular} "
                f"{winact_outcome_color(outcome)}{outcome}{COLOR_RESET}")

    if tag == "WINACT_DONE":
        action = launcher_field(msg, "action")
        total = launcher_field(msg, "total_ms")
        outcome = launcher_field(msg, "outcome")
        total_ms = winact_int(total)
        total_str = f"{total}ms" if total_ms is not None else "?ms"
        if outcome == "ok":
            verdict = (f"{COLOR_WARN}ok (partial: an AX op failed){COLOR_RESET}"
                       if partial else f"{COLOR_OK}ok{COLOR_RESET}")
        else:
            err_m = re.search(r"\berr=(?P<err>.*)$", msg)
            detail = f": {err_m.group('err')}" if err_m else ""
            verdict = f"{COLOR_FAIL}FAILED{detail}{COLOR_RESET}"
        return (f"{COLOR_HOTKEY}▶ {action}{COLOR_RESET} "
                f"{winact_ms_color(total_ms or 0)}{total_str}{COLOR_RESET} {verdict}")

    return f"{tag}: {msg}"

def process_line(ts_raw, pid, tag, msg):
    global last_winner, last_cursor_pos, last_focus_pos
    global last_printed_opacities, last_printed_statuses
    global event_buffer, last_event_real_time, ghost_dump_active, dumped_windows
    global target_opacities, picker_status
    global pending_activation, last_parsed_ts, last_divergence

    last_parsed_ts = int(ts_raw)
    
    # Check timeout for pending activation relative to parsed log time
    if pending_activation:
        if last_parsed_ts - pending_activation["ts_raw"] > 600:
            confirmed = pending_activation.get("confirmed_front")
            stats["focus_ok" if confirmed else "focus_timeout"] += 1
            if not filter_plugin or pending_activation.get("source") == filter_plugin:
                target = pending_activation["title"]
                wid = pending_activation["wid"]
                resolved_ts = pending_activation["ts_raw"] + 600
                resolved_ts_str = format_timestamp(str(resolved_ts))
                if confirmed:
                    text = f"{COLOR_SUCCESS}✔ FOCUS OK{COLOR_RESET}: \"{target}\" (wid: {wid}) confirmed front; no WM focus-change event."
                    event_buffer.append((resolved_ts, resolved_ts_str, "FOCUS", "host", text))
                else:
                    text = f"{COLOR_FAIL}✖ FOCUS FAILURE{COLOR_RESET}: Timed out focusing \"{target}\" (wid: {wid}). WM ignored request."
                    event_buffer.append((resolved_ts, resolved_ts_str, "FOCUS_WARN", "host", text))
                last_event_real_time = time.time()
            pending_activation = None

    if pending_activation and tag in ("ACTIVATE_SETTLED", "ACTIVATE_KEY_FOCUS"):
        confirm_wid = re.search(r"\bwid=(?P<wid>\d+)", msg)
        if confirm_wid and confirm_wid.group("wid") == pending_activation["wid"]:
            pending_activation["confirmed_front"] = True
            
    # Filter based on flags and topics
    if not match_topic(tag, args.topic):
        return
    if args.no_ghosts and tag in ("GHOSTDUMP", "GHOSTWIN", "SUMMARY"):
        return
    if args.no_opacity and tag in ("HIDE_WIN", "SHOW_WIN"):
        return

    ts = format_timestamp(ts_raw)
    
    for m in re.finditer(r"@(?P<x>-?\d+),(?P<y>-?\d+),(?P<w>\d+)x(?P<h>\d+)", msg):
        register_monitor(m.group("x"), m.group("y"), m.group("w"), m.group("h"))
    for m in re.finditer(r"MonitorBounds \{\s*x:\s*(?P<x>[\d.]+),\s*y:\s*(?P<y>[\d.]+),\s*width:\s*(?P<w>[\d.]+),\s*height:\s*(?P<h>[\d.]+)\s*\}", msg):
        register_monitor(m.group("x"), m.group("y"), m.group("w"), m.group("h"))
    
    if tag == "PICK":
        if filter_plugin:
            return
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
                text = (f"Winner -> {winner_color}{winner.upper()}{COLOR_RESET} | "
                        f"Cursor: {cursor_mon} (age: {cage/1000.0:.2f}s {cursor_status}) | "
                        f"Focus: {focus_mon} (age: {fage/1000.0:.2f}s {focus_status})")
                event_buffer.append((int(ts_raw), ts, "PICK", "host", text))
                last_event_real_time = time.time()
                       
    elif tag == "FOCUS_WIN":
        m = re.search(r'winpos=\((?P<x>-?\d+),(?P<y>-?\d+)[^)]*\)\s+title="(?P<title>[^"]+)"', msg)
        if m:
            title = m.group("title")
            x, y = m.group("x"), m.group("y")
            short_title = title[:30] + "..." if len(title) > 30 else title
            mon = get_monitor_name(x, y)

            wid_m = re.search(r"\bwid=(?P<wid>\d+)", msg)
            focused_wid = wid_m.group("wid") if wid_m else None
            ignored_tag = f" {COLOR_WARN}(ignored){COLOR_RESET}" if re.search(r"\bignored=true", msg) else ""

            pid_m = re.search(r"\bpid=(?P<pid>\d+)", msg)
            proc_info = ""
            if pid_m:
                fpid = pid_m.group("pid")
                proc_info = f" (proc: {get_process_name(fpid)}, pid: {fpid})"

            if pending_activation:
                if filter_plugin and pending_activation.get("source") != filter_plugin:
                    return
                target = pending_activation["title"]
                req_wid = pending_activation.get("wid")
                latency = int(ts_raw) - pending_activation["ts_raw"]

                if focused_wid is not None and req_wid is not None:
                    is_match = focused_wid == req_wid
                else:
                    is_match = (target.lower() in title.lower()) or (title.lower() in target.lower())

                detect_m = re.search(r"\bdetect_lag_ms=(?P<lag>\d+)", msg)
                detect_tag = f" (detect ±{detect_m.group('lag')}ms)" if detect_m else ""

                if is_match:
                    text = (f"{COLOR_SUCCESS}✔ FOCUS SUCCESS{COLOR_RESET}: Focused \"{title}\" "
                            f"(wid: {req_wid}) in {COLOR_SUCCESS}{latency}ms{COLOR_RESET}"
                            f"{detect_tag}{ignored_tag}")
                    record_focus_ok(int(ts_raw), req_wid, latency)
                else:
                    text = (f"{COLOR_WARN}⚠ MISDIRECTED FOCUS{COLOR_RESET}: Requested \"{target}\" "
                            f"(wid: {req_wid}), but focused \"{title}\" (wid: {focused_wid}){proc_info} "
                            f"after {latency}ms.")
                    stats["focus_misdirect"] += 1

                pending_activation = None
                event_buffer.append((int(ts_raw), ts, "FOCUS", "host", text))
            else:
                if filter_plugin:
                    return
                text = f"Active window: \"{short_title}\"{proc_info} on {mon}{ignored_tag}"
                event_buffer.append((int(ts_raw), ts, "FOCUS", "host", text))
            last_event_real_time = time.time()
                   
    elif tag == "AMC":
        if filter_plugin:
            return
        m = re.search(r"active_visible=(?P<title>\S+)", msg)
        if m:
            title = m.group("title")
            comp_title = format_title_compact(title)
            text = f"Target -> {COLOR_AMC}{comp_title}{COLOR_RESET}"
            event_buffer.append((int(ts_raw), ts, "AMC", "host", text))
            last_event_real_time = time.time()

    elif tag == "HOST_EMIT_AMC":
        if filter_plugin:
            return
        m = re.search(r"new_idx=(?P<new_idx>\S+)\s+is_boot=(?P<is_boot>\S+)", msg)
        if m:
            new_idx = m.group("new_idx")
            is_boot = m.group("is_boot")
            text = f"HOST_EMIT_AMC: new_idx={new_idx} (is_boot={is_boot})"
            event_buffer.append((int(ts_raw), ts, "HOST_EMIT_AMC", "host", text))
            last_event_real_time = time.time()

    elif tag == "PLUGIN_RECV_AMC":
        m = re.search(r"monitor_idx=(?P<idx>\S+)", msg)
        if m:
            idx = m.group("idx")
            proc_name = get_process_name(pid)
            if filter_plugin and proc_name != filter_plugin:
                return
            text = f"PLUGIN_RECV_AMC: monitor_idx={idx}"
            event_buffer.append((int(ts_raw), ts, "PLUGIN_RECV_AMC", proc_name, text))
            last_event_real_time = time.time()
                  
    elif tag == "HIDE_WIN":
        m = re.search(r"title=(?P<title>\S+)\s+wid=\d+\s+path=\S+\s+opacity=(?P<opacity>[\d.]+)", msg)
        if m:
            title = m.group("title")
            opacity = float(m.group("opacity"))
            comp_title = format_title_compact(title)
            target_opacities[comp_title] = opacity
            cls = record_opacity_write(comp_title, opacity, extract_reason(msg), int(ts_raw))

            proc_name = get_process_name(pid)
            if filter_plugin and proc_name != filter_plugin:
                return

            if cls and cls[0] == "redundant":
                return

            op_color = COLOR_OK if opacity > 0.0 else COLOR_DIM
            text = f"{comp_title} -> {op_color}{opacity}{COLOR_RESET}{reason_suffix(msg)}{churn_suffix(cls)}"
            event_buffer.append((int(ts_raw), ts, "HIDE_WIN", proc_name, text))
            last_event_real_time = time.time()

    elif tag == "SHOW_WIN":
        m = re.search(r"title=(?P<title>\S+)\s+wid=\d+\s+cleared_opacity->(?P<opacity>\d+(?:\.\d+)?)", msg)
        if m:
            title = m.group("title")
            opacity = float(m.group("opacity"))
            comp_title = format_title_compact(title)
            target_opacities[comp_title] = opacity
            cls = record_opacity_write(comp_title, opacity, extract_reason(msg), int(ts_raw))

            proc_name = get_process_name(pid)
            if filter_plugin and proc_name != filter_plugin:
                return

            if cls and cls[0] == "redundant":
                return

            payload_details = ""
            payload_m = re.search(r"source=(?P<src>\d+)\s+timestamp=(?P<ts>\d+)\s+requester_active=(?P<req>\d+)", msg)
            if payload_m:
                src = payload_m.group("src")
                ts_val = payload_m.group("ts")
                req = payload_m.group("req")
                payload_details = f" {COLOR_DIM}(EWMH: source={src}, timestamp={ts_val}, active={req}){COLOR_RESET}"
            
            text = f"{comp_title} -> {COLOR_OK}{opacity}{COLOR_RESET}{reason_suffix(msg)}{payload_details}{churn_suffix(cls)}"
            event_buffer.append((int(ts_raw), ts, "SHOW_WIN", proc_name, text))
            last_event_real_time = time.time()

    elif tag == "ALT_POLL_START":
        m = re.search(r"title=(?P<title>\S+)", msg)
        title = m.group("title") if m else "alt-tab"
        comp_title = format_title_compact(title)
        
        target_opacities[comp_title] = 1.0
        set_opacity_state(comp_title, 1.0, "open", int(ts_raw))
        prefix = comp_title.split("@")[0]
        for k in list(target_opacities.keys()):
            if k.startswith(prefix) and k != comp_title:
                target_opacities[k] = 0.0
                set_opacity_state(k, 0.0, "open", int(ts_raw))
                
        proc_name = get_process_name(pid)
        if filter_plugin and proc_name != filter_plugin:
            return
            
        text = f"Opened ({comp_title})"
        event_buffer.append((int(ts_raw), ts, "TRIGGER", proc_name, text))
        last_event_real_time = time.time()

    elif tag == "DISMISS":
        m = re.search(r"from=(?P<src>\S+)(?:\s+title=(?P<title>\S+))?", msg)
        src = m.group("src") if m else "unknown"
        title = m.group("title") if (m and m.group("title")) else ""
        comp_title_str = f" ({format_title_compact(title)})" if title else ""
        src_color = COLOR_OK if ("alt-up" in src or "super-up" in src or "modifiers/" in src) else COLOR_WARN
        proc_name = get_process_name(pid)
        if filter_plugin and proc_name != filter_plugin:
            return
            
        text = f"Closed{comp_title_str} (from={src_color}{src}{COLOR_RESET})"
        event_buffer.append((int(ts_raw), ts, "DISMISS", proc_name, text))
        last_event_real_time = time.time()

    elif tag == "CYCLE":
        m = re.search(
            r'method=(?P<method>\S+)\s+from=(?P<from>\S+)\s+to=(?P<to>\S+)\s+count=(?P<count>\d+)'
            r'\s+to_app="(?P<app>[^"]*)"\s+to_title="(?P<title>[^"]*)"\s+elapsed_ms=(?P<ms>\d+)',
            msg,
        )
        if m:
            proc_name = get_process_name(pid)
            if filter_plugin and proc_name != filter_plugin:
                return

            ms = int(m.group("ms"))
            ms_color = COLOR_FAIL if ms > 100 else (COLOR_WARN if ms > 50 else COLOR_OK)
            app = m.group("app")
            title = m.group("title")
            target = f"{app}: {title}" if title else app
            text = (f"Cycle {COLOR_HOTKEY}{m.group('method')}{COLOR_RESET} "
                    f"[{m.group('from')}->{m.group('to')}/{m.group('count')}] -> {target} "
                    f"{ms_color}({ms}ms){COLOR_RESET}")
            event_buffer.append((int(ts_raw), ts, "CYCLE", proc_name, text))
            last_event_real_time = time.time()

    elif tag == "ACTIVATE_WIN":
        m = re.search(r'wid=(?P<wid>\d+)\s+title="(?P<title>[^"]+)"', msg)
        if m:
            wid = m.group("wid")
            title = m.group("title")
            
            proc_name = get_process_name(pid)
            if filter_plugin and proc_name != filter_plugin:
                return
                
            if pending_activation:
                prev_target = pending_activation["title"]
                stats["supersede"] += 1
                text = f"{COLOR_WARN}⚠ SUPERSEDED{COLOR_RESET}: New request to focus \"{title}\" arrived before focus on \"{prev_target}\" was confirmed."
                event_buffer.append((int(ts_raw), ts, "FOCUS_WARN", proc_name, text))

            stats["focus_req"] += 1
            pending_activation = {
                "ts_raw": int(ts_raw),
                "wid": wid,
                "title": title,
                "ts_str": ts,
                "source": proc_name
            }
            
            payload_details = ""
            payload_m = re.search(r"source=(?P<src>\d+)\s+timestamp=(?P<ts>\d+)\s+requester_active=(?P<req>\d+)", msg)
            if payload_m:
                src = payload_m.group("src")
                ts_val = payload_m.group("ts")
                req = payload_m.group("req")
                payload_details = f" {COLOR_DIM}(EWMH: source={src}, timestamp={ts_val}, active={req}){COLOR_RESET}"
            
            text = f"➔ ACTIVATE REQUEST: focus \"{title}\" (wid: {wid}){payload_details}"
            event_buffer.append((int(ts_raw), ts, "ACTIVATE_WIN", proc_name, text))
            last_event_real_time = time.time()
        
    elif tag == "PICKER_STALE":
        m = re.search(r"title=(?P<title>\S+)", msg)
        if m:
            title = m.group("title")
            comp_title = format_title_compact(title)
            picker_status[comp_title] = "stale"
            
            proc_name = get_process_name(pid)
            if filter_plugin and proc_name != filter_plugin:
                return
            
            if last_printed_statuses.get(comp_title) == "stale":
                return
            last_printed_statuses[comp_title] = "stale"
            
            text = f"{comp_title} -> {COLOR_WARN}STALE{COLOR_RESET}"
            event_buffer.append((int(ts_raw), ts, "STATUS", proc_name, text))
            last_event_real_time = time.time()

    elif tag == "PICKER_READY":
        m = re.search(r"title=(?P<title>\S+)", msg)
        if m:
            title = m.group("title")
            comp_title = format_title_compact(title)
            picker_status[comp_title] = "ready"
            
            proc_name = get_process_name(pid)
            if filter_plugin and proc_name != filter_plugin:
                return
            
            if last_printed_statuses.get(comp_title) == "ready":
                return
            last_printed_statuses[comp_title] = "ready"
            
            text = f"{comp_title} -> {COLOR_OK}READY{COLOR_RESET}"
            event_buffer.append((int(ts_raw), ts, "STATUS", proc_name, text))
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
                sample_ts, title, opacity, role, map_state, owner_pid, actual_x, actual_y, actual_w, actual_h = w_info
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
                
                # Apply filter to dump windows
                if filter_plugin and proc_name != filter_plugin:
                    continue
                
                proc_suffix = f"/{proc_name}" if proc_name != "unknown" else ""
                status_suffix = f"[{picker_status.get(comp_title, 'stale')}]"
                
                if opacity > 0.0:
                    plugin_wins.setdefault(proc_name, []).append((comp_title, opacity, map_suffix))
                    
                    if role == "ghost":
                        active_ghosts.append(f"{comp_title}{proc_suffix}({opacity}{map_suffix}){status_suffix}")
                    elif role == "live" and ("alt-tab" in title or "launcher" in title):
                        active_pickers.append(f"{comp_title}{proc_suffix}({opacity}{map_suffix}){status_suffix}")
                    elif role == "invisible":
                        inactive_visible.append(f"{comp_title}{proc_suffix}({opacity}{map_suffix}){status_suffix}")
                
                expected_opacity = target_opacities.get(comp_title, 0.0)
                is_actually_hidden = (map_state != "viewable" or opacity <= 0.01)
                is_expected_hidden = (expected_opacity <= 0.01)
                
                written = opacity_state.get(comp_title)
                is_stale_sample = written is not None and sample_ts < written["ts"]
                if not (is_actually_hidden and is_expected_hidden) and not is_stale_sample:
                    if abs(opacity - expected_opacity) > 0.01:
                        divergence_msgs.append(
                            f"{comp_title} opacity is {opacity}{map_suffix}, expected {expected_opacity}{write_attribution(comp_title, int(ts_raw))}"
                        )

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
            
            for proc, wins in plugin_wins.items():
                if proc != "unknown" and len(wins) > 1:
                    comp_wins = [f"{t}({op}{ms}){write_attribution(t, int(ts_raw))}" for t, op, ms in wins]
                    inactive_visible.append(f"Multiple active {proc}: {', '.join(comp_wins)}")
                        
            status = COLOR_OK + "OK" + COLOR_RESET
            all_divergences = inactive_visible + divergence_msgs
            if all_divergences:
                status = COLOR_FAIL + f"DIVERGENCE: {', '.join(all_divergences)}" + COLOR_RESET
                divergence_key = ", ".join(sorted(all_divergences))
                if divergence_key != last_divergence:
                    stats["divergence"] += 1
                    last_divergence = divergence_key
            else:
                last_divergence = None
                
            active_parts = []
            if active_ghosts:
                active_parts.append(f"Active Ghost: {', '.join(active_ghosts)}")
            if active_pickers:
                active_parts.append(f"Active Picker: {', '.join(active_pickers)}")
                
            text = f"{' | '.join(active_parts) or 'No Active Win'} | {status}"
            
            if event_buffer and event_buffer[-1][2] == "SUMMARY" and event_buffer[-1][4] == text:
                pass
            else:
                event_buffer.append((int(ts_raw), ts, "SUMMARY", "host", text))
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
            dumped_windows.append((int(ts_raw), title, opacity, role, map_state, owner_pid, x, y, w, h))

    elif tag == "PUBLISH":
        m = re.search(r'idx=(?P<idx>\d+)\s+"(?P<name>[^"]+)"\s+is_boot=(?P<is_boot>\w+)\s+->\s+delivered=\[(?P<delivered>[^\]]*)\]\s+missed=\[(?P<missed>[^\]]*)\]', msg)
        if m:
            idx = m.group("idx")
            name = m.group("name")
            is_boot = m.group("is_boot")
            delivered = m.group("delivered")
            missed = m.group("missed")
            
            if filter_plugin and (filter_plugin not in delivered and filter_plugin not in missed):
                return
                
            text = f"PUBLISH AMC idx={idx} \"{name}\" is_boot={is_boot} -> delivered=[{delivered}] missed=[{missed}]"
            event_buffer.append((int(ts_raw), ts, "PUBLISH", "host", text))
            last_event_real_time = time.time()

    elif tag == "SUBSCRIBE":
        m = re.search(r'plugin=(?P<plugin>\S+)\s+interests=\[(?P<interests>[^\]]*)\](?:\s+->\s+host\s+sticky-replay\s+AMC\s+idx=(?P<idx>\d+))?', msg)
        if m:
            plugin = m.group("plugin")
            interests = m.group("interests")
            idx = m.group("idx")
            
            if filter_plugin and plugin != filter_plugin:
                return
                
            replay_str = f" -> host sticky-replay AMC idx={idx}" if idx else ""
            text = f"SUBSCRIBE plugin={plugin} interests=[{interests}]{replay_str}"
            event_buffer.append((int(ts_raw), ts, "SUBSCRIBE", plugin, text))
            last_event_real_time = time.time()

    elif tag == "RECV":
        m = re.search(r'AMC\s+idx=(?P<idx>\d+)\s+"(?P<name>[^"]+)"\s+src=(?P<src>\w+)', msg)
        if m:
            idx = m.group("idx")
            name = m.group("name")
            src = m.group("src")
            
            proc_name = get_process_name(pid)
            if filter_plugin and proc_name != filter_plugin:
                return
                
            text = f"RECV AMC idx={idx} \"{name}\" src={src}"
            event_buffer.append((int(ts_raw), ts, "RECV", proc_name, text))
            last_event_real_time = time.time()

    elif tag == "ACTIVATE":
        m = re.search(r'#(?P<seq>\d+)\s+wid=(?P<wid>\d+)\s+title="(?P<title>[^"]+)"\s+source=(?P<src>\d+)(?:\((?P<src_name>[^)]+)\))?\s+sent_ts=(?P<sent_ts>\d+)\s+requestor_active=(?P<req_active>\S+)', msg)
        if m:
            seq = m.group("seq")
            wid = m.group("wid")
            title = m.group("title")
            src = m.group("src")
            src_name = m.group("src_name") or str(src)
            sent_ts = m.group("sent_ts")
            req_active = m.group("req_active")
            
            proc_name = get_process_name(pid)
            if filter_plugin and proc_name != filter_plugin:
                return
                
            pending_activation = {
                "seq": seq,
                "ts_raw": int(ts_raw),
                "wid": wid,
                "title": title,
                "ts_str": ts,
                "source": proc_name
            }
            
            text = f"➔ ACTIVATE #{seq}: focus \"{title}\" (wid: {wid}) source={src}({src_name}) sent_ts={sent_ts} requestor_active={req_active}"
            event_buffer.append((int(ts_raw), ts, "ACTIVATE", proc_name, text))
            last_event_real_time = time.time()

    elif tag == "WM_RECEIVE":
        m = re.search(r'#(?P<seq>\d+)\s+target="(?P<target>[^"]+)"\s+_NET_WM_USER_TIME=(?P<user_time>\d+)\s+msg_ts=(?P<msg_ts>\d+)\s+->\s+(?P<comparison>\S+)\s+\((?P<status>[^)]+)\)', msg)
        if m:
            seq = m.group("seq")
            target = m.group("target")
            user_time = m.group("user_time")
            msg_ts = m.group("msg_ts")
            comparison = m.group("comparison")
            status = m.group("status")
            
            if pending_activation and pending_activation.get("seq") == seq:
                if filter_plugin and pending_activation.get("source") != filter_plugin:
                    return
            elif filter_plugin:
                return
                
            text = f"WM_RECEIVE #{seq}: target=\"{target}\" _NET_WM_USER_TIME={user_time} msg_ts={msg_ts} -> {comparison} ({status})"
            event_buffer.append((int(ts_raw), ts, "WM_RECEIVE", "host", text))
            last_event_real_time = time.time()

    elif tag == "FOCUS":
        m = re.search(r'#(?P<seq>\d+)\s+result=(?P<result>\w+)\s+active_after=(?P<active_after>\S+)\s+demands_attention=(?P<demands_attention>\w+)\s+elapsed=(?P<elapsed>\d+)ms', msg)
        if m:
            seq = m.group("seq")
            result = m.group("result")
            active_after = m.group("active_after")
            demands_attention = m.group("demands_attention")
            elapsed = m.group("elapsed")
            
            if pending_activation and pending_activation.get("seq") == seq:
                if filter_plugin and pending_activation.get("source") != filter_plugin:
                    return
                pending_activation = None
            elif filter_plugin:
                return
                
            res_color = COLOR_SUCCESS if result == "OK" else COLOR_FAIL
            if result == "OK":
                text = f"{res_color}✔ FOCUS #{seq} SUCCESS{COLOR_RESET}: active_after={active_after} elapsed={elapsed}ms"
            else:
                text = f"{res_color}✖ FOCUS #{seq} {result}{COLOR_RESET}: active_after={active_after} demands_attention={demands_attention} elapsed={elapsed}ms"
            event_buffer.append((int(ts_raw), ts, "FOCUS", "host", text))
            last_event_real_time = time.time()

    elif tag == "LEGEND":
        m = re.search(r'mon\s+(?P<details>.*)', msg)
        if m:
            details = m.group("details")
            text = f"LEGEND mon {details}"
            event_buffer.append((int(ts_raw), ts, "LEGEND", "host", text))
            last_event_real_time = time.time()

    elif tag == "MARK":
        m = re.search(r'message="(?P<msg>[^"]+)"', msg)
        if m:
            msg_text = m.group("msg")
            width = 60
            text = f"\n{COLOR_HOTKEY}" + "─" * ((width - len(msg_text) - 8) // 2) + f" MARK: {msg_text} " + "─" * ((width - len(msg_text) - 9) // 2) + f"{COLOR_RESET}\n"
            event_buffer.append((int(ts_raw), ts, "MARK", "host", text))
            last_event_real_time = time.time()

    elif tag.startswith("LAUNCHER_"):
        proc_name = get_process_name(pid)
        if filter_plugin and proc_name != filter_plugin:
            return
        text = format_launcher_event(tag, msg)
        event_buffer.append((int(ts_raw), ts, tag, proc_name, text))
        last_event_real_time = time.time()

    elif tag == "GHOST_DUMP":
        m = re.search(
            r'ctx=\((?P<ctx>[^)]*)\)\s+title="(?P<title>[^"]*)"\s+alpha=(?P<alpha>[\d.]+)'
            r'\s+level=(?P<level>-?\d+)\s+mouse_ignored=(?P<mi>\w+)\s+frame=(?P<frame>\S+)',
            msg,
        )
        if m:
            proc_name = get_process_name(pid)
            if filter_plugin and proc_name != filter_plugin:
                return
            ctx = m.group("ctx")
            title = m.group("title")
            alpha = float(m.group("alpha"))
            level = int(m.group("level"))
            mouse_ignored = m.group("mi") == "true"
            is_ghost = title.startswith("qol-")
            wrong_level = is_ghost and level == 0
            opaque_outside_show = (
                is_ghost and alpha >= 1.0 and not mouse_ignored and not ctx.startswith("show")
            )
            text = (f"\"{title}\" alpha={alpha} level={level} "
                    f"mouse_ignored={m.group('mi')} {m.group('frame')} ({ctx})")
            if wrong_level or opaque_outside_show:
                stats["divergence"] += 1
                why = "ghost at normal window level" if wrong_level else "opaque clickable ghost outside show"
                text = f"{COLOR_FAIL}⚠ DIVERGENCE ({why}): {text}{COLOR_RESET}"
            event_buffer.append((int(ts_raw), ts, "GHOST_DUMP", proc_name, text))
            last_event_real_time = time.time()

    elif tag.startswith("PROFILE_"):
        if filter_plugin and filter_plugin != "profile":
            return
        color = COLOR_FAIL if "outcome=include" in msg and "entry_kind=symlink" in msg else ""
        reset = COLOR_RESET if color else ""
        text = f"{color}{tag}: {msg}{reset}"
        event_buffer.append((int(ts_raw), ts, tag, "profile", text))
        last_event_real_time = time.time()

    elif tag.startswith("WORLD_"):
        if filter_plugin and filter_plugin != "world":
            return
        is_bad = (
            "outcome=reject" in msg
            or "outcome=skip" in msg
            or "reason=already_dived" in msg
            or "visible_slots=0" in msg
        )
        color = COLOR_FAIL if is_bad else ""
        reset = COLOR_RESET if color else ""
        text = f"{color}{tag}: {msg}{reset}"
        event_buffer.append((int(ts_raw), ts, tag, "world", text))
        last_event_real_time = time.time()

    elif tag.startswith("WINACT_"):
        if filter_plugin and filter_plugin != "window-actions":
            return
        partial = False
        if tag == "WINACT_AX" and "outcome=fail" in msg:
            winact_fail_pids.add(pid)
        elif tag == "WINACT_DONE":
            partial = pid in winact_fail_pids
            winact_fail_pids.discard(pid)
        text = format_winact_event(tag, msg, partial)
        event_buffer.append((int(ts_raw), ts, tag, "window-actions", text))
        last_event_real_time = time.time()

    else:
        proc_name = get_process_name(pid)
        if filter_plugin and proc_name != filter_plugin:
            return
        color = COLOR_FAIL if "DIVERGENCE" in msg else ""
        reset = COLOR_RESET if color else ""
        text = f"{color}{tag}: {msg}{reset}"
        event_buffer.append((int(ts_raw), ts, tag, proc_name, text))
        last_event_real_time = time.time()

def flush_buffer():
    global event_buffer, last_printed_state_summary
    if not event_buffer:
        return
        
    unique_events = []
    seen_texts = set()
    for ts_val, ts_str, tag, source, text in event_buffer:
        if args.grep and args.grep.lower() not in text.lower():
            continue
        if args.anomalies and not any(mk in text for mk in ANOMALY_MARKERS):
            continue
        if tag == "SUMMARY":
            if text == last_printed_state_summary:
                continue
            last_printed_state_summary = text
        if not tag.startswith("WINACT_"):
            if text in seen_texts:
                continue
            seen_texts.add(text)
        unique_events.append((ts_val, ts_str, tag, source, text))
        
    if not unique_events:
        event_buffer = []
        return
        
    t_root_ms, ts_root, _, source_root, text_root = unique_events[0]
    
    t_last_ms = unique_events[-1][0]
    span_ms = t_last_ms - t_root_ms
    latency_str = f" {COLOR_TIME}(span: {span_ms}ms){COLOR_RESET}" if span_ms > 0 else ""
    
    n = len(unique_events)
    src_color = hash_color(source_root)
    src_tag = f"{src_color}[{source_root}]{COLOR_RESET} "
    
    if n == 1:
        print(f"{COLOR_TIME}[{ts_root}]{COLOR_RESET} ── {src_tag}{text_root}{latency_str}")
    else:
        print(f"{COLOR_TIME}[{ts_root}]{COLOR_RESET} ┌── {src_tag}{text_root}{latency_str}")
        for idx in range(1, n):
            _, ts, _, source, text = unique_events[idx]
            connector = "└── " if idx == n - 1 else "├── "
            c_color = hash_color(source)
            c_tag = f"{c_color}[{source}]{COLOR_RESET} "
            print(f"{COLOR_TIME}[{ts}]{COLOR_RESET} │   {connector}{c_tag}{text}")
            
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

REPLAY_GAP_MS = 120

def replay_log(f, start_ts):
    prev_ts = None
    for line in f:
        ts, pid, tag, msg = parse_line(line.strip())
        if not (ts and pid and tag and msg):
            continue
        if start_ts > 0 and int(ts) < start_ts:
            continue
        if prev_ts is not None and int(ts) - prev_ts > REPLAY_GAP_MS and event_buffer:
            flush_buffer()
        process_line(ts, pid, tag, msg)
        prev_ts = int(ts)

def main():
    global last_event_real_time, pending_activation
    if args.mark:
        try:
            with open(LOG_FILE, "a") as f:
                ts = int(time.time() * 1000)
                pid = os.getpid()
                f.write(f"{ts} pid={pid} MARK message=\"{args.mark}\"\n")
            print(f"Injected marker: {args.mark}")
            sys.exit(0)
        except Exception as e:
            print(f"Error injecting marker: {e}")
            sys.exit(1)

    if not os.path.exists(LOG_FILE):
        print(f"Error: Log file {LOG_FILE} does not exist yet. Please run the daemon.")
        sys.exit(1)
        
    if args.focus_only:
        filter_str = f" for {COLOR_OK}{filter_plugin}{COLOR_HEADER}" if filter_plugin else ""
        print(f"{COLOR_HEADER}Tailing /tmp/qol-altmon.log (focus-only mode{filter_str})...{COLOR_RESET}")
    elif filter_plugin:
        print(f"{COLOR_HEADER}Tailing /tmp/qol-altmon.log filtering for {COLOR_OK}{filter_plugin}{COLOR_HEADER}...{COLOR_RESET}")
    else:
        print(f"{COLOR_HEADER}Tailing /tmp/qol-altmon.log (system runtime trace)...{COLOR_RESET}")
    print(f"{COLOR_DIM}Aggregating transitions into beautiful call tree structures with latencies.{COLOR_RESET}\n")
    
    query_initial_monitors()
    
    # Parse --since parameter
    start_ts = 0
    if args.since:
        m = re.match(r"(?P<val>\d+)(?P<unit>[smh])?", args.since)
        if m:
            val = int(m.group("val"))
            unit = m.group("unit") or "s"
            mult = {"s": 1, "m": 60, "h": 3600}[unit]
            start_ts = int((time.time() - val * mult) * 1000)

    f = open(LOG_FILE, "r")
    if args.replay:
        f.seek(0)
        print(f"{COLOR_DIM}Replaying full log...{COLOR_RESET}\n")
        replay_log(f, start_ts)
        flush_buffer()
        print_stats()
        print_waste()
        sys.exit(0)
    if start_ts > 0:
        print(f"Reading events since {args.since}...")
    else:
        f.seek(0, os.SEEK_END)

    try:
        while True:
            # Check timeout for pending activation in real-time
            if pending_activation:
                cur_ms = int(time.time() * 1000)
                if cur_ms - pending_activation["ts_raw"] > 600:
                    if not filter_plugin or pending_activation.get("source") == filter_plugin:
                        target = pending_activation["title"]
                        wid = pending_activation["wid"]
                        resolved_ts = pending_activation["ts_raw"] + 600
                        resolved_ts_str = format_timestamp(str(resolved_ts))
                        if pending_activation.get("confirmed_front"):
                            text = f"{COLOR_SUCCESS}✔ FOCUS OK{COLOR_RESET}: \"{target}\" (wid: {wid}) confirmed front; no WM focus-change event."
                            event_buffer.append((resolved_ts, resolved_ts_str, "FOCUS", "host", text))
                        else:
                            text = f"{COLOR_FAIL}✖ FOCUS FAILURE{COLOR_RESET}: Timed out focusing \"{target}\" (wid: {wid}). WM ignored request."
                            event_buffer.append((resolved_ts, resolved_ts_str, "FOCUS_WARN", "host", text))
                        last_event_real_time = time.time()
                    pending_activation = None

            if event_buffer and (time.time() - last_event_real_time > 0.08):
                flush_buffer()
                
            line = f.readline()
            if not line:
                time.sleep(0.01)
                continue
            ts, pid, tag, msg = parse_line(line.strip())
            if ts and pid and tag and msg:
                if start_ts > 0 and int(ts) < start_ts:
                    continue
                process_line(ts, pid, tag, msg)
    except KeyboardInterrupt:
        flush_buffer()
        print_stats()
        print_waste()
        print(f"\n{COLOR_HEADER}Exiting tailer.{COLOR_RESET}")
        sys.exit(0)

if __name__ == "__main__":
    main()
