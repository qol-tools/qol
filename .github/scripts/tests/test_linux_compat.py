import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import linux_compat as lc


class LinuxCompatibilityTests(unittest.TestCase):
    def test_glibc_versions_are_ordered_numerically_and_unknown_abis_fail_closed(self):
        for text, expected in [
            ("", (0, 0, 0)),
            ("Name: GLIBC_2.9 Name: GLIBC_2.39", (2, 39, 0)),
            ("Name: GLIBC_2.2.5 Name: GLIBC_2.34", (2, 34, 0)),
            ("Name: GLIBC_2.43 Name: GLIBC_2.39", (2, 43, 0)),
            ("Name: GLIBC_ABI_DT_RELR", (2, 36, 0)),
            ("Name: GLIBCXX_3.4.30", (0, 0, 0)),
        ]:
            with self.subTest(text=text):
                self.assertEqual(lc.glibc_requirement(text), expected)
        for name in ["GLIBC_PRIVATE", "GLIBC_ABI_FUTURE", "GLIBC_invalid"]:
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, name):
                lc.glibc_requirement(f"Name: {name}")

    def test_release_gate_accepts_the_baseline_and_rejects_newer_or_invalid_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary"
            binary.write_bytes(b"\x7fELFfixture")
            for version, accepted in [("2.2.5", True), ("2.39", True), ("2.39.1", False), ("2.43", False), ("3.0", False)]:
                with self.subTest(version=version):
                    output = subprocess.CompletedProcess([], 0, f"Name: GLIBC_{version}", "")
                    with patch.object(lc.subprocess, "run", return_value=output):
                        if accepted:
                            evidence = lc.verify_binary(binary)
                            self.assertEqual(len(evidence["sha256"]), 64)
                            continue
                        with self.assertRaisesRegex(ValueError, "release baseline"):
                            lc.verify_binary(binary)
            binary.write_text("not ELF")
            with self.assertRaisesRegex(ValueError, "expected an ELF"):
                lc.verify_binary(binary)

    def test_every_packaged_host_binary_and_the_declared_plugin_binary_are_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text('[workspace]\nmembers = ["apps/*", "plugins/*"]\n')
            host = root / "apps/tray"
            host.mkdir(parents=True)
            (host / "Cargo.toml").write_text('[package]\nname = "qol-tray"\n[package.metadata.deb]\nassets = [["target/release/host", "usr/bin/", "755"], ["target/release/helper", "usr/lib/", "755"], ["asset.png", "usr/share/", "644"]]\n')
            self.assertEqual(lc.release_binaries(root, "qol-tray", None), [root / "target/release/host", root / "target/release/helper"])
            plugin = root / "plugins/fixture"
            plugin.mkdir(parents=True)
            (plugin / "Cargo.toml").write_text('[package]\nname = "fixture-package"\n')
            (plugin / "plugin.toml").write_text('[[dependencies.binaries]]\nname = "actual-binary"\n')
            self.assertEqual(lc.release_binaries(root, "fixture-package", "x86_64-unknown-linux-gnu"), [root / "target/x86_64-unknown-linux-gnu/release/actual-binary"])

    @unittest.skipUnless(sys.platform == "linux" and shutil.which("readelf"), "requires Linux readelf")
    def test_real_elf_with_unavailable_glibc_is_rejected(self):
        original = Path(shutil.which("true")).read_bytes()
        info = subprocess.run(["readelf", "--version-info", shutil.which("true")], capture_output=True, text=True, check=True).stdout
        import re

        versions = re.findall(r"Name: (GLIBC_\d+\.\d+(?:\.\d+)?)", info)
        self.assertTrue(versions)
        version = max(versions, key=len).encode()
        incompatible = b"GLIBC_9.99".ljust(len(version), b"9")
        self.assertEqual(len(version), len(incompatible))
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "incompatible"
            binary.write_bytes(original.replace(version, incompatible))
            binary.chmod(0o755)
            failed = subprocess.run([str(binary)], capture_output=True, check=False)
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn(b"GLIBC_", failed.stderr)
            with self.assertRaisesRegex(ValueError, "release baseline"):
                lc.verify_binary(binary)


if __name__ == "__main__":
    unittest.main()
