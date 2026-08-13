import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CARGO_GATE = re.compile(r"\bcargo (?:build|test|clippy|check)\b")


class ReleaseWorkflowContractTests(unittest.TestCase):
    def test_versioning_waits_for_main_ci(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()

        self.assertIn("workflow_run:", workflow)
        self.assertIn("conclusion == 'success'", workflow)
        self.assertIn(
            "SOURCE_CI_RUN_ID: ${{ github.event.workflow_run.id }}", workflow
        )
        self.assertIn('args+=(--run-id "${SOURCE_CI_RUN_ID}")', workflow)
        self.assertNotIn("push:\n    branches: [main]", workflow)

    def test_candidates_are_attested_before_tags_are_created(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()

        attest = workflow.index("release_candidate.py attest")
        tag = workflow.index('git tag "${tag}"')
        self.assertLess(attest, tag)

    def test_candidate_release_set_remains_atomic(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()

        required = [
            "needs.plugin_candidate.result == 'success'",
            "needs.qol_tray_linux_candidate.result == 'success'",
            "needs.qol_tray_macos_candidate.result == 'success'",
            'git push --atomic origin "${new_tags[@]}"',
        ]
        for contract in required:
            self.assertIn(contract, workflow)

    def test_release_workflows_verify_exact_candidate(self):
        for name in ["release.yml", "qol-tray-release.yml"]:
            with self.subTest(workflow=name):
                workflow = (ROOT / ".github/workflows" / name).read_text()
                verify = workflow.index("release_candidate.py verify")
                build = workflow.index("release_candidate.py build")
                self.assertLess(verify, build)

    def test_ci_runs_release_profile_builds(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        self.assertIn("cargo build --release --locked $BUILD_ARGS", workflow)
        self.assertIn("RUSTFLAGS: -D warnings", workflow)
        self.assertNotIn("debug-assertions", workflow)
        self.assertIn("timeout-minutes:", workflow)

    def test_ci_matches_the_release_pipeline_lockfile_strictness(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        lenient = [
            line.strip()
            for line in workflow.splitlines()
            if CARGO_GATE.search(line)
            and not line.strip().startswith("#")
            and "--locked" not in line
        ]
        self.assertEqual(
            lenient,
            [],
            "every CI cargo gate must pass --locked so a stale Cargo.lock cannot "
            "reach the --locked release pipeline",
        )

    def test_ci_asserts_lockfile_freshness_with_a_full_resolve(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        self.assertIn("cargo metadata --locked --format-version 1", workflow)
        self.assertNotIn(
            "cargo metadata --locked --no-deps",
            workflow,
            "--no-deps skips resolution and exits 0 against a stale lockfile",
        )

    def test_windows_dev_build_tests_follow_the_affected_plan(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        for contract in [
            "windows_dev_build: ${{ steps.affected.outputs.windows_dev_build }}",
            "fromJSON(needs.plan.outputs.windows_dev_build || 'false')",
            "if: ${{ fromJSON(needs.plan.outputs.windows_dev_build) }}",
            "run: cargo test --locked -p qol-dev-build --all-targets",
        ]:
            self.assertIn(contract, workflow)

    def test_plugin_publish_never_claims_latest(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()

        self.assertIn(
            'gh release create "$tag" --draft --title "$tag" --generate-notes --latest=false release_files/*',
            workflow,
        )
        self.assertIn('gh release edit "$tag" --draft=false --latest=false', workflow)
        self.assertNotIn("--latest=true", workflow)

    def test_tray_publish_claims_latest(self):
        workflow = (ROOT / ".github/workflows/qol-tray-release.yml").read_text()

        self.assertIn(
            'gh release edit "${RELEASE_TAG}" --draft=false --latest=true',
            workflow,
        )
        self.assertNotIn("--latest=false", workflow)

    def test_release_creation_steps_carry_an_explicit_latest_policy(self):
        for workflow_path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            with self.subTest(workflow=workflow_path.name):
                workflow = workflow_path.read_text()
                lines = workflow.splitlines()
                for index, line in enumerate(lines):
                    if "gh release create" in line:
                        self.assertIn(
                            "--latest",
                            line,
                            f"release creation must carry an explicit latest flag: "
                            f"{workflow_path.name}:{index + 1}",
                        )
                    if "action-gh-release" in line:
                        block = lines[index + 1 : index + 12]
                        self.assertTrue(
                            any("make_latest:" in b for b in block),
                            f"action-gh-release step must declare make_latest: "
                            f"{workflow_path.name}:{index + 1}",
                        )


if __name__ == "__main__":
    unittest.main()
