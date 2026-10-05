import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("plugin_artifact", SCRIPTS / "plugin_artifact.py")
pa = importlib.util.module_from_spec(_SPEC)
sys.modules["plugin_artifact"] = pa
_SPEC.loader.exec_module(pa)


def git(cwd: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True)


def write_plugin(root: Path, plugin_dir: str, plugin_id: str) -> None:
    folder = root / "plugins" / plugin_dir
    folder.mkdir(parents=True)
    (folder / "plugin.toml").write_text(f'[plugin]\nid = "{plugin_id}"\nversion = "1.0.0"\n')


class PushArgsTests(unittest.TestCase):
    def test_tree_layer_leads_and_assets_follow_in_name_order(self):
        args = pa.push_args(
            "ghcr.io/o/plugins:qol-shot-v1.0.0",
            "shot",
            "https://github.com/o/qol",
            Path("/tmp/plugin.toml"),
            "qol-shot-tree.tar.gz",
            ["qol-shot-macos-aarch64", "IBM-Plex-OFL.txt", "qol-shot-linux-x86_64"],
        )
        self.assertEqual(args[:2], ["push", "ghcr.io/o/plugins:qol-shot-v1.0.0"])
        self.assertIn(f"{pa.SOURCE_ANNOTATION}=https://github.com/o/qol", args)
        self.assertIn(f"{pa.DIR_ANNOTATION}=shot", args)
        self.assertIn(f"/tmp/plugin.toml:{pa.CONFIG_MEDIA_TYPE}", args)
        self.assertEqual(
            args[-4:],
            [
                f"qol-shot-tree.tar.gz:{pa.TREE_MEDIA_TYPE}",
                f"IBM-Plex-OFL.txt:{pa.ASSET_MEDIA_TYPE}",
                f"qol-shot-linux-x86_64:{pa.ASSET_MEDIA_TYPE}",
                f"qol-shot-macos-aarch64:{pa.ASSET_MEDIA_TYPE}",
            ],
        )


class BackfillTagsTests(unittest.TestCase):
    def test_keeps_release_tags_of_current_plugins_only(self):
        tags = [
            "qol-alt-tab-v0.89.0",
            "qol-alt-tab-v0.88.0",
            "plugin-alt-tab-v0.40.0",
            "qol-tray-v3.82.0",
            "qol-shot-v1.77.0",
            "not-a-release",
        ]
        selected = pa.backfill_tags(tags, {"qol-alt-tab", "qol-shot"})
        self.assertEqual(
            selected, ["qol-alt-tab-v0.88.0", "qol-alt-tab-v0.89.0", "qol-shot-v1.77.0"]
        )


class PluginDirTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.cwd = os.getcwd()
        git(self.root, "init", "-q")
        write_plugin(self.root, "alt-tab", "qol-alt-tab")
        write_plugin(self.root, "shot", "qol-shot")
        (self.root / "plugins" / "notes").mkdir()
        (self.root / "plugins" / "notes" / "README.md").write_text("no manifest")
        git(self.root, "add", ".")
        git(self.root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init")
        os.chdir(self.root)

    def tearDown(self):
        os.chdir(self.cwd)
        self.tmp.cleanup()

    def test_finds_the_directory_that_declares_the_id(self):
        self.assertEqual(pa.plugin_dir_at("HEAD", "qol-shot"), "shot")
        self.assertEqual(pa.plugin_dir_at("HEAD", "qol-alt-tab"), "alt-tab")

    def test_refuses_an_id_no_directory_declares(self):
        with self.assertRaises(ValueError):
            pa.plugin_dir_at("HEAD", "qol-missing")

    def test_current_plugin_ids_reads_the_working_tree(self):
        self.assertEqual(pa.current_plugin_ids(self.root), {"qol-alt-tab", "qol-shot"})


if __name__ == "__main__":
    unittest.main()
