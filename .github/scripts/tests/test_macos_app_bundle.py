import os
import plistlib
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import macos_app_bundle as bundle  # noqa: E402


class BundleTests(unittest.TestCase):
    def test_the_plist_names_the_tray_and_its_version(self):
        plist = plistlib.loads(bundle.info_plist("3.2.1").encode())
        self.assertEqual(plist["CFBundleIdentifier"], "com.qol-tools.qol-tray")
        self.assertEqual(plist["CFBundleExecutable"], bundle.BINARY)
        self.assertEqual(plist["CFBundleVersion"], "3.2.1")
        self.assertEqual(plist["CFBundleShortVersionString"], "3.2.1")
        self.assertTrue(plist["LSUIElement"])
        for key in ("NSBluetoothAlwaysUsageDescription", "NSMicrophoneUsageDescription", "NSAudioCaptureUsageDescription"):
            self.assertTrue(plist[key], key)

    def test_assemble_lays_out_an_executable_bundle_and_replaces_an_old_one(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            binary = root / "built"
            binary.write_bytes(b"binary")
            app = root / "QoL Tray.app"
            (app / "Contents" / "stale").mkdir(parents=True)
            bundle.assemble(binary, "3.2.1", app)
            executable = app / "Contents" / "MacOS" / bundle.BINARY
            self.assertEqual(executable.read_bytes(), b"binary")
            self.assertTrue(os.access(executable, os.X_OK))
            self.assertTrue((app / "Contents" / "Resources" / "qol-tray.icns").is_file())
            self.assertTrue((app / "Contents" / "Resources" / "IBM-Plex-OFL.txt").is_file())
            self.assertFalse((app / "Contents" / "stale").exists())
            plist = plistlib.loads((app / "Contents" / "Info.plist").read_bytes())
            self.assertEqual(plist["CFBundleVersion"], "3.2.1")


if __name__ == "__main__":
    unittest.main()
