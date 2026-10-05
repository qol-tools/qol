import importlib.util
import sys
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("plugin_index", SCRIPTS / "plugin_index.py")
pi = importlib.util.module_from_spec(_SPEC)
sys.modules["plugin_index"] = pi
_SPEC.loader.exec_module(pi)

LOCATION = {"url": "https://ghcr.io", "repository": "qol-tools/plugins"}


def manifest(*layers: dict, plugin_dir: str = "alt-tab") -> dict:
    return {"layers": list(layers), "annotations": {pi.DIR_ANNOTATION: plugin_dir}}


def tree_layer(digest: str = "sha256:tree") -> dict:
    return {"mediaType": pi.TREE_MEDIA_TYPE, "digest": digest, "size": 10}


def asset_layer(name: str, digest: str) -> dict:
    return {
        "mediaType": "application/octet-stream",
        "digest": digest,
        "size": 20,
        "annotations": {pi.TITLE_ANNOTATION: name},
    }


def plugin_toml(plugin_id: str, version: str) -> str:
    return f'[plugin]\nid = "{plugin_id}"\nversion = "{version}"\n'


def fake_fetch(tag: str) -> dict:
    plugin_id, _ = pi.parse_tag(tag)
    version = tag.removeprefix(f"{plugin_id}-v")
    return pi.version_entry(
        f"sha256:{tag}",
        manifest(tree_layer(), asset_layer(f"{plugin_id}-linux-x86_64", f"sha256:bin-{tag}")),
        plugin_toml(plugin_id, version),
    )


class VersionEntryTests(unittest.TestCase):
    def test_splits_tree_from_assets_by_media_type(self):
        entry = pi.version_entry(
            "sha256:m",
            manifest(
                tree_layer("sha256:t"),
                asset_layer("qol-alt-tab-linux-x86_64", "sha256:a"),
                asset_layer("IBM-Plex-OFL.txt", "sha256:o"),
            ),
            plugin_toml("qol-alt-tab", "1.0.0"),
        )
        self.assertEqual(entry["manifest_digest"], "sha256:m")
        self.assertEqual(entry["dir"], "alt-tab")
        self.assertEqual(entry["tree"], {"digest": "sha256:t", "size": 10})
        self.assertEqual(
            entry["assets"],
            {
                "qol-alt-tab-linux-x86_64": {"digest": "sha256:a", "size": 20},
                "IBM-Plex-OFL.txt": {"digest": "sha256:o", "size": 20},
            },
        )

    def test_requires_exactly_one_tree_layer(self):
        cases = [
            manifest(asset_layer("a", "sha256:a")),
            manifest(tree_layer("sha256:t1"), tree_layer("sha256:t2")),
        ]
        for case in cases:
            with self.subTest(layers=len(case["layers"])):
                with self.assertRaises(ValueError):
                    pi.version_entry("sha256:m", case, plugin_toml("p", "1.0.0"))


    def test_requires_a_plain_plugin_directory_name(self):
        for plugin_dir in ["", "plugins/alt-tab", "..", ".hidden"]:
            with self.subTest(plugin_dir=plugin_dir):
                with self.assertRaises(ValueError):
                    pi.version_entry(
                        "sha256:m",
                        manifest(tree_layer(), plugin_dir=plugin_dir),
                        plugin_toml("p", "1.0.0"),
                    )


class BuildIndexTests(unittest.TestCase):
    def test_keeps_newest_versions_and_marks_semver_latest(self):
        tags = [
            "qol-alt-tab-v0.9.0",
            "qol-alt-tab-v0.10.0",
            "qol-alt-tab-v0.2.0",
            "qol-shot-v1.0.0",
            "sha256-abc.sig",
        ]
        index = pi.build_index(tags, fake_fetch, LOCATION, keep=2, serial=7)
        self.assertEqual(index["schema"], pi.SCHEMA)
        self.assertEqual(index["serial"], 7)
        self.assertEqual(index["registry"], LOCATION)
        alt_tab = index["plugins"]["qol-alt-tab"]
        self.assertEqual(alt_tab["latest"], "0.10.0")
        self.assertEqual(sorted(alt_tab["versions"]), ["0.10.0", "0.9.0"])
        self.assertEqual(index["plugins"]["qol-shot"]["latest"], "1.0.0")

    def test_refuses_an_artifact_whose_manifest_names_another_release(self):
        def mismatched(tag: str) -> dict:
            entry = fake_fetch(tag)
            entry["plugin_toml"] = plugin_toml("qol-alt-tab", "9.9.9")
            return entry

        with self.assertRaises(ValueError):
            pi.build_index(["qol-alt-tab-v1.0.0"], mismatched, LOCATION, keep=3, serial=1)

    def test_empty_registry_builds_an_empty_index(self):
        index = pi.build_index([], fake_fetch, LOCATION, keep=3, serial=1)
        self.assertEqual(index["plugins"], {})


class RegistryLocationTests(unittest.TestCase):
    def test_splits_host_from_repository(self):
        cases = [
            (False, {"url": "https://ghcr.io", "repository": "qol-tools/plugins"}),
            (True, {"url": "http://ghcr.io", "repository": "qol-tools/plugins"}),
        ]
        for plain_http, expected in cases:
            with self.subTest(plain_http=plain_http):
                registry = pi.Registry("ghcr.io/qol-tools/plugins", plain_http)
                self.assertEqual(registry.location(), expected)


if __name__ == "__main__":
    unittest.main()
