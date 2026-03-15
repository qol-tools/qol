#!/bin/bash
cd /home/user/git/qol-tools/plugin-lights
RUST_LOG=trace QOL_TRAY_DAEMON_SOCKET=/tmp/plugin-lights-debug.sock ./target/debug/plugin-lights 2>&1 | tee /tmp/lights-debug.log
