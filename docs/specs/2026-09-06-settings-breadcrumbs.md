# Required settings breadcrumb metadata

## Request and acceptance

The native Shortcut and Hotkey editors currently change their private mode without contributing that mode to the settings header. The user requires an architectural contract, including compile-time enforcement for statically declared settings destinations.

Acceptance criteria:

1. Opening Add Shortcut displays `QoL Settings > Shortcuts > Add Shortcut`.
2. Opening Add Hotkey displays `QoL Settings > Hotkeys > Add Hotkey`.
3. Existing-record editors display `Edit Shortcut` and `Edit Hotkey` respectively. A plugin-managed shortcut editor also receives an explicit, nonempty destination.
4. Escape and successful save return the breadcrumb to the list. Validation failure leaves the editor breadcrumb visible. Source changes, retained reopen, direct Add entry, and rail navigation never display another source's stale editor path.
5. Contract-driven nested cards continue to display their resolved labels, including the user's Alt Tab / Switchable Panels example.
6. A custom settings view cannot be registered without implementing a required breadcrumb interface. No default implementation, optional metadata, public raw-view constructor, or empty-string fallback bypasses the contract.
7. A new native editor mode requires an exhaustive breadcrumb mapping. Rendering and the breadcrumb read the same mode; no separately maintained push/pop breadcrumb stack for custom editors.
8. Dynamic destination labels reject blank and whitespace-only strings before navigation changes. Root is represented explicitly, never by an empty destination label.
9. Existing keyboard navigation, selection, draft discard on Escape, save behavior, and styling remain intact. No polling for breadcrumbs.
10. Central build, tests, Clippy, formatting, compile-contract probes, and disposable-guest behavior checks pass before acceptance and local delivery.

The browser dashboard and validation-toast timeout are outside this change. The toast issue belongs to the separate `settings-validation-toast` architect task. Do not change toast policy or notification plumbing beyond retaining the existing notifier while migrating the custom-view constructor.

## Design decision

The shared settings panel owns breadcrumb presentation. A custom view supplies destination metadata from its current navigation state through a required trait. The shared wrapper observes the entity's existing notifications to invalidate the parent header. This removes the missing information boundary without introducing an imperative breadcrumb state that can diverge from editor state.

The existing `CustomPanelView` wrapper currently exposes raw view and focus fields. Replace that unchecked construction seam with a generic, typed constructor. Its fields must be private outside the settings-panel implementation. Preserve the factory alias unless a concrete framework constraint requires an explained adjustment.

## Shared API

Implement the following public contract in `libs/qol-gpui/src/settings_panel/navigation.rs`, exported from `settings_panel/mod.rs`:

- `SettingsDestination`: private label storage, `Clone + Debug + PartialEq + Eq`, no `Default`.
- `SettingsDestination::new(label: impl Into<String>) -> anyhow::Result<Self>` validates and normalizes a nonempty trimmed dynamic label.
- `SettingsDestination::label(&self) -> &str` exposes the validated text.
- `SettingsDestination::from_static(label: &'static str) -> Self` is a checked `const fn` suitable for native destination constants. Its label must be an explicit argument. Use a `Cow<'static, str>` representation or another const-compatible representation; validate static labels without silently substituting another label. Declare native destinations as constants so invalid static metadata is rejected during compilation.
- `CustomSettingsBreadcrumbs`: required `fn settings_breadcrumbs(&self) -> Vec<SettingsDestination>` with no default body. An empty vector explicitly represents the custom source's root/list state; individual destinations can never have empty labels.
- `CustomPanelInvalidator = Rc<dyn Fn(&mut gpui::App)>`.

In `settings_panel/mod.rs`:

- Add required `on_change: CustomPanelInvalidator` to `CustomPanelContext`.
- Replace public `CustomPanelView` field construction with `CustomPanelView::new<T>(entity: gpui::Entity<T>, on_change: CustomPanelInvalidator, cx: &mut gpui::App) -> Self` where `T: gpui::Render + gpui::Focusable + CustomSettingsBreadcrumbs + 'static`.
- The wrapper retains its entity/view, focus handle, typed-to-erased breadcrumb reader, and observation subscription. The observation reads no duplicate navigation state and does not poll.
- Expose only the accessors needed by the containing panel. Keep raw fields private to the containing module if that avoids unnecessary accessors.
- A registration missing the breadcrumb trait must fail at the constructor bound.

In `settings_panel/view.rs`:

- Supply `on_change` using the existing weak parent entity. Defer parent notification when needed to avoid a reentrant entity update during child state changes or construction.
- Derive the custom tail from the currently displayed custom view through the wrapper and `&App`. Thread application context into `trail` / `render_band` as necessary.
- Make header selection follow the displayed source and navigation mode. A rail/root screen must not inherit a hidden child editor's tail. Retained child state must have matching metadata when shown again.
- Replace optional `Level.title` with an explicit private root/card representation whose card variant holds `SettingsDestination`. Remove the `level.title.clone().unwrap_or_default()` breadcrumb fallback.
- Require validated destination metadata in `push_card` and every non-root card creation path. `push_card(destination: SettingsDestination, child: Level)` is an acceptable enforcement signature; it must always install the supplied destination rather than accepting a root/no-metadata child as the final stack entry.
- Validate dynamic row labels before mutating navigation. Use existing panel error presentation on invalid metadata, without a panic or an invented title. Preserve valid card behavior and existing displayed labels.
- Add a focused existing-trace event if needed to prove source and destination transitions. Trace identifiers and labels must avoid user-entered content; static destination kinds and depth are sufficient. Do not create a new polling trace.

## Native editor integration

In `apps/qol-tray/src/settings_surface/platform/native_tools/mod.rs`:

- Destructure the new `on_change` context field.
- Register the entity with `CustomPanelView::new(view, on_change, cx)` instead of a raw struct literal.

In `native_tools/view.rs`:

- Implement `CustomSettingsBreadcrumbs` for `NativeToolsView`.
- Derive the result from the same `Mode` and draft identity used by rendering, through an exhaustive mapping with no wildcard branch for editor variants.
- List mode returns the source root path. Blank shortcut/hotkey drafts return Add Shortcut/Add Hotkey. Existing drafts return Edit Shortcut/Edit Hotkey. Managed shortcuts must be explicitly covered; they may use Edit Shortcut if that accurately describes the existing editor.
- Use static destination constants rather than accepting an arbitrary label at each transition. The renderer and breadcrumb must not use competing route enums.
- Ensure existing state-change notifications invalidate the header through the shared observer on open, close, load-driven direct Add, and save completion. Do not manually publish separate Push/Pop breadcrumb events.
- Preserve draft and pending-action semantics. If exiting to the rail intentionally retains editor state, show its matching tail only when that child is displayed again. Do not redesign pending-save cancellation as a breadcrumb fix.

## Ownership and integration

One grouped implementation delivery, with disjoint file ownership:

### Lane settings-breadcrumbs-contract

- `libs/qol-gpui/src/settings_panel/navigation.rs` (new)
- `libs/qol-gpui/src/settings_panel/mod.rs`
- `libs/qol-gpui/src/settings_panel/view.rs`
- `libs/qol-gpui/tests/fixtures/settings_breadcrumbs/complete_contract.rs` (new)
- `libs/qol-gpui/tests/fixtures/settings_breadcrumbs/missing_contract.rs` (new)
- `libs/qol-gpui/tests/fixtures/settings_breadcrumbs/missing_destination.rs` (new)

The positive fixture registers a generic entity with all required bounds. The missing-contract fixture uses an otherwise identical generic registration without `CustomSettingsBreadcrumbs` and must fail specifically at that bound. The missing-destination fixture attempts metadata-free construction and must fail at the required/private constructor boundary. Fixtures contain no code comments and no standalone Cargo manifests or new dependencies. The architect compiles these probes against the built library artifacts centrally.

### Lane settings-breadcrumbs-native

- `apps/qol-tray/src/settings_surface/platform/native_tools/mod.rs`
- `apps/qol-tray/src/settings_surface/platform/native_tools/view.rs`

Neither lane edits this spec, the other lane's files, Cargo manifests, lockfiles, shared toast files, or any unrelated source. Report a necessary ownership expansion rather than making it. Both lanes are edit-only: no builds, tests, lint, format, git commands, installs, host desktop operations, or additional lane spawning.

## Audit before implementation

The architect ran workspace-wide fixed-string searches in this worktree:

- `CustomPanelView {`: definition in shared `settings_panel/mod.rs`, construction in native `native_tools/mod.rs`. Both are owned above.
- `level.title.clone().unwrap_or_default()`: one breadcrumb assembly hit in shared `settings_panel/view.rs`, owned above.

No existing display-label literal is designated for removal or renaming. Existing root/source/card labels stay unchanged. Add/Edit destination labels are additions. If implementation proposes removing or changing an existing literal, stop and report its exact literal or assembled static fragments; the architect must audit every workspace hit and assign ownership before that change.

## Verification plan

Architect only, after both lanes return and personal diff review:

1. Read every changed file and inspect the API for unchecked raw construction, optional labels, default breadcrumb methods, duplicate custom-navigation state, and wildcard mappings that bypass new mode exhaustiveness.
2. Run one central round of `cargo run -q -p qol -- check` covering affected build/test/Clippy/format and repository checks. Include a release build for the affected native consumer and shared library as required by repository rules.
3. Compile the positive fixture against the selected built artifacts, then compile each negative fixture and require the intended contract error. A missing dependency or unrelated syntax error is not a passing negative test.
4. Unit tests cover label rejection/normalization; native list/add/edit/managed mode mapping; existing source/card trail composition; and root-versus-child handling. Test actual policy or transitions, not an implementation-mirroring copy of string concatenation.
5. Use a disposable Mint guest with this worktree's artifacts. Exercise Shortcuts list -> Add -> Escape, existing Shortcut -> Edit -> Escape, Hotkeys list -> Add -> failed save -> Escape, existing Hotkey -> Edit, direct Add activation from a cold settings host, source switching while a child exists, retained close/reopen, and a contract-driven nested card such as Switchable Panels.
6. Capture header screenshots and available decision-level traces. Verify the exact two user-reported Add paths and adjacent Back/list behavior. Observe at least one successful save returning to its list, using only disposable guest data. Never drive the host desktop.
7. If a gate fails, return a bounded correction round to the owner, then review and gate the new integrated round. No lane self-acceptance.
8. Preserve guest reports, shut down owned guests, squash locally into main according to git-trees, run the required loop close and retrospective, and do not push or reinstall on the host without further authorization.

## Risks requiring personal review

- Parent invalidation must not reenter a leased child entity or keep a destroyed settings panel alive.
- Retained custom views can hold editor state while another source is visible; breadcrumb selection must match what is displayed.
- Dynamic card metadata validation must fail before changing stack depth or selection.
- The public typed constructor and exhaustive native mapping enforce the declared navigation contract. They cannot prove the semantic correctness of arbitrary consumer-supplied words; acceptance tests verify the concrete user paths.

## First review correction round

The repository gate stopped at native formatting before compilation. The native lane must apply the rustfmt-requested single-line ADD_HOTKEY_DESTINATION declaration and multiline single_label length assertion manually.

Personal diff review found open_list_card and open_object_array_card retain references borrowed from self.level() across a mutable self.card_destination call, then reuse those references. The contract lane must validate before taking the long-lived row-control borrow or finish extracting owned child data first, preserving validation before navigation mutation.

Static whitespace validation currently recognizes only ASCII whitespace while dynamic validation uses str::trim. Make both reject whitespace-only labels including Unicode White_Space; retain const evaluation for native constants. Add focused coverage and one additional owned fixture, libs/qol-gpui/tests/fixtures/settings_breadcrumbs/blank_static_destination.rs, whose const destination must fail evaluation for a Unicode-whitespace-only label. No new dependencies or changes to existing display literals are authorized.

The contract lane must inspect its own changed code for additional Rust ownership errors without executing checks. The architect will rerun the integrated gate after both corrections return.

## Compile correction and guest-route scout

The next central gate passed formatting but failed rust-build: recursion limit reached while expanding #[test] in native_tools/view.rs:1719. The breadcrumb_tests module imports super::* from a module importing gpui::*, bringing in GPUI's test attribute. Replace the test-module glob with explicit required symbols; do not raise recursion limits or change production behavior. Ownership remains native_tools/mod.rs and view.rs.

A parallel read-only guest-route scout may inspect apps/qol-tray/src/settings_surface/, apps/qol-tray/src/web/, apps/qol-tray/src/shortcuts/, tools/qol-cli/src/commands/, and existing target/qol-shot-status-verification/ evidence. It must report the precise supported guest action routes for core Shortcuts/Hotkeys list and direct Add, guest authentication discovery, and keyboard steps for the acceptance cases. No file edits or command execution beyond reading/searching these files; no build, test, lint, format, git, host desktop control, or guest launch. The architect runs the actual verification.

## Direct takeover

The user requested stopping the lanes and completing the fix directly. Both lane reports were collected, the remaining scout was interrupted, and all subsequent implementation and verification belong to the primary agent. No further lanes are authorized for this task.
