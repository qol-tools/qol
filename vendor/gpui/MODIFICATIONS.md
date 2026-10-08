# Modifications

This is gpui 0.2.2 from crates.io, changed by qol-tools:

- `src/elements/text.rs`: each measure of a text element truncates a copy of its runs, so a narrow measure no longer shortens what a later, wider layout draws. Zed fixed the same bug upstream in zed-industries/zed#45122.
- `src/taffy.rs`: the grid repeat names its float literals `f32`, as Zed does upstream, so newer compilers do not reject the fallback to `f32`.
- `src/window.rs`: when the platform reports a resize or move while the app is already borrowed, the window reads its new bounds once that update returns instead of dropping them, and several reports before then share one deferred read. qol code sets a native window frame synchronously inside an update, and gpui used to keep the stale viewport and scale and log `RefCell already borrowed`.
- `src/platform/linux/x11/window.rs`: a new window is created at its requested origin (no 2 px shift) and WM_NORMAL_HINTS carries that origin and size as user-specified with static gravity, so Muffin and other window managers map it there instead of placing it and moving it after the first map, and a restored inner bounds lands exactly. Upstream gpui still sets only the minimum size; this is an upstream candidate for zed-industries/zed.
- The `examples` and `docs` folders and the `[[example]]` targets are removed.
- `src/app/test_context.rs` and `src/platform/test/window.rs`: expose the fake window handle and add simulated moves so workspace regression tests can deliver resize/move callbacks during an app update. The tests cover deferred bounds, coalescing, subsequent immediate updates, and closing a window before deferred work runs.
