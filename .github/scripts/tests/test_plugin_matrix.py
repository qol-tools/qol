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
        write_plugin(self.root, "alt-tab", "plugin-alt-tab")

        outputs = pm.release_outputs(self.root, "plugin-alt-tab", "1.2.3")

        self.assertEqual(outputs["crate_dir"], (self.root / "plugins/alt-tab").as_posix())
        self.assertEqual(outputs["package"], "fixture-package")

    def test_rejects_duplicate_declared_identity(self):
        write_plugin(self.root, "first", "plugin-duplicate")
        write_plugin(self.root, "second", "plugin-duplicate")

        with self.assertRaisesRegex(ValueError, "multiple"):
            pm.plugin_crate_dir(self.root, "plugin-duplicate")

    def test_rejects_version_mismatch(self):
        write_plugin(self.root, "alt-tab", "plugin-alt-tab", "2.0.0")

        with self.assertRaisesRegex(ValueError, "tag expects version"):
            pm.release_outputs(self.root, "plugin-alt-tab", "1.2.3")


if __name__ == "__main__":
    unittest.main()
