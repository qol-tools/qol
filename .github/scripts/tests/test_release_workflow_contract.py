import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CARGO_GATE = re.compile(r"\bcargo (?:build|test|clippy|check|nextest run)\b")


def named_step(document, name, indent):
    start = document.index(f"{indent}- name: {name}\n")
    end = document.find(f"\n{indent}- ", start + 1)
    return document[start:end] if end != -1 else document[start:]


class ReleaseWorkflowContractTests(unittest.TestCase):
    def test_compiler_cache_preserves_dependency_keys_and_remains_opt_in(self):
        action = (ROOT / ".github/actions/rust-setup/action.yml").read_text()
        inputs = action.split("outputs:", 1)[0]
        compiler_input = inputs.split("  compiler-cache:\n", 1)[1]
        self.assertIn('default: "false"', compiler_input)
        self.assertLess(
            action.index("uses: Swatinem/rust-cache@"),
            action.index("RUSTC_WRAPPER=sccache"),
        )
        self.assertIn("SCCACHE_CACHE_SIZE=${COMPILER_CACHE_CAPACITY}", action)
        self.assertRegex(action, r"tool: sccache@\d+\.\d+\.\d+")
        self.assertNotIn("SCCACHE_GHA_ENABLED", action)
        for name in ["Configure", "Install", "Restore", "Enable"]:
            with self.subTest(step=name):
                step = named_step(action, f"{name} compiler cache", "    ")
                self.assertIn("if: inputs.compiler-cache == 'true'", step)

    def test_only_ci_check_opts_into_compiler_cache(self):
        enabled = []
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            for match in re.finditer(r'''^\s+compiler-cache:\s*["']?true["']?\s*$''', path.read_text(), re.M):
                enabled.append(path.name)
        self.assertEqual(enabled, ["ci.yml"])
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        self.assertIn('compiler-cache: "true"', check)

    def test_compiler_cache_contract_flows_from_the_policy_owner(self):
        action = (ROOT / ".github/actions/rust-setup/action.yml").read_text()
        configure = named_step(action, "Configure compiler cache", "    ")
        self.assertIn('cache_prune.py --compiler-cache-config >> "${GITHUB_OUTPUT}"', configure)
        restore = named_step(action, "Restore compiler cache", "    ")
        for field, output in [("path", "path"), ("key", "key"), ("restore-keys", "prefix")]:
            self.assertIn(f"{field}: ${{{{ steps.compiler-config.outputs.{output} }}}}", restore)
        enable = named_step(action, "Enable compiler cache", "    ")
        self.assertIn("COMPILER_CACHE_PATH: ${{ steps.compiler-config.outputs.path }}", enable)
        self.assertIn("COMPILER_CACHE_CAPACITY: ${{ steps.compiler-config.outputs.capacity }}", enable)
        self.assertIn("SCCACHE_DIR=${COMPILER_CACHE_PATH}", enable)
        outputs = action.split("outputs:\n", 1)[1].split("runs:\n", 1)[0]
        self.assertIn("compiler-cache-path:", outputs)
        self.assertIn("value: ${{ steps.compiler-config.outputs.path }}", outputs)

    def test_compiler_cache_saves_only_main_and_publishes_statistics(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  compiler-cache-retire:\n", 1)[0]
        self.assertIn('compiler-cache: "true"', check)
        for name in ["Flush compiler cache", "Save compiler cache"]:
            with self.subTest(step=name):
                step = named_step(check, name, "      ")
                for contract in [
                    "!cancelled()",
                    "github.ref == 'refs/heads/main'",
                    "steps.rust_setup.outcome == 'success'",
                    "steps.rust_setup.outputs.compiler-cache-hit != 'true'",
                ]:
                    self.assertIn(contract, step)
        self.assertIn("sccache --stop-server || true", named_step(check, "Flush compiler cache", "      "))
        save = named_step(check, "Save compiler cache", "      ")
        self.assertIn("key: ${{ steps.rust_setup.outputs.compiler-cache-key }}", save)
        self.assertIn("path: ${{ steps.rust_setup.outputs.compiler-cache-path }}", save)
        for name in ["Record compiler cache statistics", "Upload compiler cache statistics"]:
            with self.subTest(step=name):
                self.assertIn("!cancelled()", named_step(check, name, "      "))
        upload = named_step(check, "Upload compiler cache statistics", "      ")
        self.assertIn("overwrite: true", upload)
        self.assertIn("sccache --show-stats --stats-format=json", check)
        self.assertIn("name: compiler-cache-${{ matrix.os }}", check)
        self.assertLess(check.index("cargo nextest run"), check.index("Flush compiler cache"))
        self.assertLess(check.index("Flush compiler cache"), check.index("Save compiler cache"))

    def test_build_jobs_never_hold_a_cache_deletion_token(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  compiler-cache-retire:\n", 1)[0]
        self.assertNotIn("actions: write", check)
        self.assertNotIn("github.token", check)
        self.assertNotIn("cache_prune.py", check)
        retire = workflow.split("  compiler-cache-retire:\n", 1)[1].split("  process-windows:\n", 1)[0]
        self.assertIn("needs: check", retire)
        self.assertIn("github.ref == 'refs/heads/main'", retire)
        self.assertIn("needs.check.result != 'skipped'", retire)
        self.assertIn("permissions:\n      contents: read\n      actions: write\n", retire)
        self.assertIn("persist-credentials: false", retire)
        self.assertIn("sparse-checkout: .github/scripts", retire)
        self.assertNotIn("rust-setup", retire)
        self.assertNotRegex(retire, CARGO_GATE)
        step = named_step(retire, "Retire superseded compiler archives", "      ")
        self.assertIn("continue-on-error: true", step)
        self.assertIn('--retire-compiler-caches "$GITHUB_SHA" --ref "$GITHUB_REF"', step)

    def test_clippy_does_not_use_the_uncacheable_driver_wrapper(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn('RUSTC_WRAPPER: ""', named_step(workflow, "Clippy", "      "))

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

    def test_shared_crate_releases_wait_for_binary_probes(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        release = (ROOT / ".github/workflows/release.yml").read_text()

        self.assertIn("needs: [probe_plan, probe]", workflow)
        self.assertIn("needs.probe.result == 'skipped'", workflow)
        self.assertLess(workflow.index("release_probe.py"), workflow.index("--probe-reports"))
        for contract in [
            "cache-key: plugin-release-${{ matrix.target }}",
            "RUSTFLAGS: -D warnings ${{ matrix.os == 'ubuntu-latest' && '-C link-arg=-fuse-ld=lld' || '' }}",
            "--kind plugin",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, release)
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

        self.assertIn("merge_group:", workflow)
        self.assertIn("cargo check --release --locked $BUILD_ARGS", workflow)
        self.assertIn("cargo build --release --locked $BUILD_ARGS", workflow)
        self.assertIn("github.event_name != 'pull_request'", workflow)
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

    def test_nextest_preserves_affected_scope_and_documentation_on_each_platform(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check_job = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        test_steps = re.findall(r"      - name: Test\n(.*?)(?=\n      - name:|\Z)", check_job, re.S)
        self.assertEqual(len(test_steps), 2)
        self.assertRegex(check_job, r"tool: cargo-nextest@\d+\.\d+\.\d+")
        for step in test_steps:
            with self.subTest(step=step.splitlines()[0]):
                self.assertIn("TEST_ARGS: ${{ matrix.test_args }}", step)
                self.assertIn("DOCTEST: ${{ matrix.doctest }}", step)
                self.assertIn('if [ "$DOCTEST" = "true" ]; then', step)
                self.assertIn(
                    "cargo nextest run --locked $TEST_ARGS --no-fail-fast --retries 0 --no-tests=pass --ignore-default-filter",
                    step,
                )
                self.assertIn("cargo test --locked $TEST_ARGS --doc", step)
                self.assertLess(step.index("cargo nextest run"), step.index("cargo test"))

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
