#!/usr/bin/env python3
"""Assemble the QoL Tray app bundle around a built qol-tray binary.

The release and the live test ship the same bundle, so both build it here:
the binary under Contents/MacOS, the icon and font license under
Contents/Resources, and the Info.plist that names the bundle and declares the
usage descriptions macOS asks for before the tray may use Bluetooth, the
microphone or the sound output. Signing stays with the workflow.
"""
import argparse
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BINARY = "qol-tray"
ICON = ROOT / "apps/tray/assets/qol-tray.icns"
FONT_LICENSE = ROOT / "libs/gpui/assets/fonts/OFL.txt"

PLIST = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
<key>CFBundleIdentifier</key>
<string>com.qol-tools.qol-tray</string>
<key>CFBundleName</key>
<string>QoL Tray</string>
<key>CFBundleExecutable</key>
<string>qol-tray</string>
<key>CFBundleIconFile</key>
<string>qol-tray</string>
<key>CFBundleVersion</key>
<string>{version}</string>
<key>CFBundleShortVersionString</key>
<string>{version}</string>
<key>LSUIElement</key>
<true/>
<key>LSMinimumSystemVersion</key>
<string>11.0</string>
<key>NSBluetoothAlwaysUsageDescription</key>
<string>QoL Tray reconnects the Bluetooth devices you choose and lists nearby devices to pair.</string>
<key>NSMicrophoneUsageDescription</key>
<string>QoL Tray shows how loud your microphone is in the Sound settings.</string>
<key>NSAudioCaptureUsageDescription</key>
<string>QoL Tray shows how loud the sound playing on your Mac is in the Sound settings.</string>
</dict>
</plist>
"""


def info_plist(version: str) -> str:
    return PLIST.format(version=version)


def assemble(binary: Path, version: str, app: Path) -> Path:
    contents = app / "Contents"
    executables = contents / "MacOS"
    resources = contents / "Resources"
    if app.exists():
        shutil.rmtree(app)
    executables.mkdir(parents=True)
    resources.mkdir(parents=True)
    shutil.copyfile(binary, executables / BINARY)
    (executables / BINARY).chmod(0o755)
    shutil.copyfile(ICON, resources / "qol-tray.icns")
    shutil.copyfile(FONT_LICENSE, resources / "IBM-Plex-OFL.txt")
    (contents / "Info.plist").write_text(info_plist(version), encoding="utf-8")
    return app


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--app", required=True, type=Path)
    args = parser.parse_args()
    assemble(args.binary, args.version, args.app)
    return 0


if __name__ == "__main__":
    sys.exit(main())
