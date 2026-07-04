# qol-cli

The `qol` CLI: dev console (`qol dev`), emu workflows (`qol emu`), doctor, trace.

## Dev console design rules

- **One frame, breadcrumb sign.** `draw()` renders a single bordered frame
  (the `qol dev` panel) and hands its inner `Rect` to the view. The view paints
  its content straight into that rect via `view_content` (lists) or a direct
  `Paragraph` (streams) - it does NOT wrap itself in a second box. The frame's
  `Sign` is the breadcrumb `breadcrumb(dash)` - **location only**
  (`qol dev · <page>`, `qol dev · emu · <id>`; ancestors dim, leaf bold) plus
  the global `ARMED`/`RELOADING` flag. No live status (line counts, follow,
  age) in the title - that is a separate concept. A new view is a `draw_*`
  that renders content into its rect.
- **Page description is a dim header, not title chrome.** `page_description`
  returns a short static blurb per view; `page_header` renders it as a dim line
  at the top of the content rect (then a blank row) and returns the shrunk rect
  the view draws into. Pages without a description (dashboard, emu-detail) get
  the rect unchanged. Keep blurbs short so the title sign never has to carry
  them.
- **`SignBox` is for genuine sub-panes only.** The bordered+titled `SignBox`
  is reserved for a real nested pane inside a page (the run.log pane in
  `draw_emu_detail`) and the floating keys badge - never to wrap a whole page.
  `Sign`, `SignBox`, and the breadcrumb all compose the same centered tab, so
  signs cannot drift apart.
- **Size scrolling from the rect, gap-aware.** List pages window against the
  full inner rect via `list_capacity(area.height)` (which divides by the item
  gap); the run.log pane windows against `SignBox::capacity` for its own
  chrome. Never `area.height - N` arithmetic.
- **One line per list item.** Detail lines below an item are reserved for
  failure states; healthy items earn exactly one line. Static facts that never
  change between renders (paths, versions) belong in `qol emu doctor` or the
  empty state, not on every frame.
- **One accent source.** `draw()` derives the frame accent once
  (`frame_accent`: red RELOADING > orange WORKTREE > yellow ARMED >
  `BASE_ACCENT` green) and publishes it via `render_util::set_frame_accent`;
  every "healthy/brand green" in any view reads `render_util::accent()`.
  Never write `Color::Green` in render code - hardcoding it splits the color
  source and that element stops following the frame state. Red/yellow error
  and warning semantics stay literal; only the green family routes through
  the accent. `frame_accent` itself is the ONE place that must NOT read
  `accent()`: its fallback is the `BASE_ACCENT` constant. Reading the
  thread-local there feeds the published value back into itself and latches
  the previous frame's color permanently.
- **One worktree source.** The persisted worktree selection lives in exactly
  one place: the active-worktree marker (`qol_dev_build::tray` marker IO),
  shared with the web UI and the tray boot contract. Argv is a transient
  directive (`<branch>` writes the marker, `--base` clears it, absent follows
  it) and `Dash.worktree_selection` is transient session intent. Anything that
  builds or launches a tray binary resolves its target FROM the marker
  (`marker_tray_target`); never resolve from argv or console state directly,
  and never clear the marker except on an explicit `--base`.
