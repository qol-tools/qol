import argparse
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
        release = {
            "plugin_dir": "shot",
            "revision": "abc123",
            "assets": {
                "qol-shot-macos-aarch64": "sha256:m",
                "IBM-Plex-OFL.txt": "sha256:o",
                "qol-shot-linux-x86_64": "sha256:l",
            },
        }
        args = pa.push_args(
            "ghcr.io/o/plugins:qol-shot-v1.0.0",
            release,
            "https://github.com/o/qol",
            Path("/tmp/plugin.toml"),
            "qol-shot-tree.tar.gz",
        )
        self.assertEqual(args[:2], ["push", "ghcr.io/o/plugins:qol-shot-v1.0.0"])
        self.assertIn(f"{pa.SOURCE_ANNOTATION}=https://github.com/o/qol", args)
        self.assertIn(f"{pa.REVISION_ANNOTATION}=abc123", args)
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

    def test_push_reruns_only_over_the_same_release(self):
        files = self.root / "release_files"
        files.mkdir()
        (files / "qol-shot-linux-x86_64").write_bytes(b"binary")
        release = pa.release_content("qol-shot-v1.0.0", "HEAD", files)
        same = published_manifest(release)
        other_binary = published_manifest({**release, "assets": {"qol-shot-linux-x86_64": "sha256:x"}})
        other_revision = published_manifest({**release, "revision": "0" * 40})
        cases = [
            ("absent", None, 0, 1),
            ("same release", same, 0, 0),
            ("other binary", other_binary, 1, 0),
            ("other revision", other_revision, 1, 0),
        ]
        for label, published, status, pushes in cases:
            with self.subTest(label):
                registry = FakeRegistry(published)
                original = pa.Registry
                pa.Registry = lambda reference, plain_http: registry
                try:
                    args = argparse.Namespace(
                        registry="r",
                        plain_http=False,
                        tag="qol-shot-v1.0.0",
                        revision="HEAD",
                        files=files,
                        source="https://github.com/o/qol",
                    )
                    self.assertEqual(pa.push(args), status)
                finally:
                    pa.Registry = original
                self.assertEqual(len(registry.pushes), pushes)


class FakeRegistry:
    reference = "r"

    def __init__(self, published: dict | None) -> None:
        self.published = published
        self.pushes: list[list[str]] = []

    def published_manifest(self, tag: str) -> dict | None:
        return self.published

    def oras(self, args: list[str], cwd: Path | None = None) -> str:
        self.pushes.append(args)
        return ""


def published_manifest(release: dict) -> dict:
    assets = [
        {"mediaType": pa.ASSET_MEDIA_TYPE, "digest": digest, "annotations": {pa.TITLE_ANNOTATION: name}}
        for name, digest in release["assets"].items()
    ]
    return {
        "annotations": {pa.REVISION_ANNOTATION: release["revision"]},
        "config": {"digest": pa.sha256_digest(release["config"].encode())},
        "layers": [{"mediaType": pa.TREE_MEDIA_TYPE, "digest": "sha256:t"}, *assets],
    }


if __name__ == "__main__":
    unittest.main()
