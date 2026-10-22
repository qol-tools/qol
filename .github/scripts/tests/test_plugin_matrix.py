import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "plugin_matrix", Path(__file__).resolve().parents[1] / "plugin_matrix.py"
)
pm = importlib.util.module_from_spec(_SPEC)
sys.modules["plugin_matrix"] = pm
_SPEC.loader.exec_module(pm)


def write_plugin(root: Path, directory: str, plugin_id: str, version: str = "1.2.3"):
    crate = root / "plugins" / directory
    crate.mkdir(parents=True)
    (crate / "plugin.toml").write_text(
        f'[plugin]\nid = "{plugin_id}"\nname = "Fixture"\ndescription = ""\n'
        f'version = "{version}"\nplatforms = ["linux"]\n'
    )
    (crate / "Cargo.toml").write_text(
        f'[package]\nname = "fixture-package"\nversion = "{version}"\n'
    )


class PluginMatrixTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def test_resolves_manifest_identity_independently_from_directory(self):
        write_plugin(self.root, "alt-tab", "qol-alt-tab")

        outputs = pm.release_outputs(self.root, "qol-alt-tab", "1.2.3")

        self.assertEqual(outputs["crate_dir"], (self.root / "plugins/alt-tab").as_posix())
        self.assertEqual(outputs["package"], "fixture-package")

    def test_rejects_duplicate_declared_identity(self):
        write_plugin(self.root, "first", "plugin-duplicate")
        write_plugin(self.root, "second", "plugin-duplicate")

        with self.assertRaisesRegex(ValueError, "multiple"):
            pm.plugin_crate_dir(self.root, "plugin-duplicate")

    def test_rejects_version_mismatch(self):
        write_plugin(self.root, "alt-tab", "qol-alt-tab", "2.0.0")

        with self.assertRaisesRegex(ValueError, "tag expects version"):
            pm.release_outputs(self.root, "qol-alt-tab", "1.2.3")

    def test_rejects_a_platform_it_cannot_build(self):
        cases = [
            ('["linux", "macOS"]', r"unknown platforms \['macOS'\] for qol-alt-tab"),
            ('["darwin"]', r"unknown platforms \['darwin'\]"),
            ("[]", "no buildable platforms"),
        ]
        for platforms, error in cases:
            with self.subTest(platforms=platforms), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                write_plugin(root, "alt-tab", "qol-alt-tab")
                manifest = root / "plugins/alt-tab/plugin.toml"
                manifest.write_text(manifest.read_text().replace('["linux"]', platforms))
                with self.assertRaisesRegex(ValueError, error):
                    pm.release_outputs(root, "qol-alt-tab", "1.2.3")

    def test_groups_every_plugin_of_a_target_into_one_row(self):
        entries = [
            {"os": "ubuntu-latest", "target": "linux", "package": "a", "tag": "a-v1.0.0"},
            {"os": "macos-latest", "target": "mac", "package": "a", "tag": "a-v1.0.0"},
            {"os": "ubuntu-latest", "target": "linux", "package": "b", "tag": "b-v2.0.0"},
        ]

        self.assertEqual(
            pm.group_by_target(entries),
            [
                {
                    "os": "ubuntu-latest",
                    "target": "linux",
                    "plugins": [
                        {"package": "a", "tag": "a-v1.0.0"},
                        {"package": "b", "tag": "b-v2.0.0"},
                    ],
                },
                {
                    "os": "macos-latest",
                    "target": "mac",
                    "plugins": [{"package": "a", "tag": "a-v1.0.0"}],
                },
            ],
        )
        self.assertEqual(pm.group_by_target([]), [])


if __name__ == "__main__":
    unittest.main()
