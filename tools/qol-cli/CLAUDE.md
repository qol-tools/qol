# qol-cli

The `qol` CLI: dev console (`qol dev`), emu workflows (`qol emu`), doctor, trace.

## Dev console design rules

- **One frame, breadcrumb sign.** `draw()` renders a single bordered frame
  (the `qol dev` panel) and hands its inner `Rect` to the view. The view paints
  its content straight into that rect via `view_content` (lists) or a direct
  `Paragraph` (streams) - it does NOT wrap itself in a second box. Each
  `draw_*` returns its breadcrumb leaf as `Vec<Span>`; `draw()` assembles
  `breadcrumb()` (`qol dev · <page> · <status>`, ancestors dim, leaf bold) and
  renders it as the frame's `Sign`. A new view is a `draw_*` that renders
  content and returns its leaf.
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
