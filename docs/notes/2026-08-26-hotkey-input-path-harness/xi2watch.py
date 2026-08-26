#!/usr/bin/env python3
# Records what the X server actually receives: RawKeyPress/RawKeyRelease with the
# source device id (17 = qol-tray-virtual-keyboard, other = the probe device directly).
import re, subprocess, sys, time
out = open(sys.argv[1], "w", buffering=1)
p = subprocess.Popen(["stdbuf", "-oL", "xinput", "test-xi2", "--root"], stdout=subprocess.PIPE, text=True, bufsize=1)
ev = src = None
for line in p.stdout:
    m = re.match(r"EVENT type \d+ \((\w+)\)", line)
    if m: ev, src = m.group(1), None; continue
    m = re.match(r"\s+device: (\d+) \((\d+)\)", line)
    if m: src = m.group(2); continue
    m = re.match(r"\s+detail: (\d+)", line)
    if m and ev in ("RawKeyPress", "RawKeyRelease"):
        out.write(f"{int(time.time()*1000)} {ev} src={src} detail={m.group(1)}\n")
